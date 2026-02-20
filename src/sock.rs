//! Unix domain socket for hook queries against the in-memory index.
//! MCP server spawns a listener thread; hook processes connect for zero-I/O queries.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Socket path: ~/.amaranthine/hook.sock
pub fn sock_path(dir: &Path) -> PathBuf {
    dir.join("hook.sock")
}

/// Start the socket listener thread. Returns the join handle.
/// Cleans up the socket file on drop via the returned guard.
pub fn start_listener(dir: &Path) -> Option<SockGuard> {
    let path = sock_path(dir);
    // Remove stale socket
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("amaranthine: sock bind failed: {e}");
            return None;
        }
    };
    // Non-blocking accept with 500ms timeout for clean shutdown
    listener.set_nonblocking(false).ok();
    let dir2 = dir.to_path_buf();
    let path2 = path.clone();
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => { handle_conn(s, &dir2); }
                Err(e) => {
                    // Check if socket file was removed (shutdown signal)
                    if !path2.exists() { break; }
                    if e.kind() != std::io::ErrorKind::WouldBlock {
                        eprintln!("amaranthine: sock accept: {e}");
                    }
                }
            }
        }
    });
    Some(SockGuard { path, _handle: handle })
}

pub struct SockGuard {
    path: PathBuf,
    _handle: std::thread::JoinHandle<()>,
}

impl Drop for SockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Handle a single hook query connection.
/// Uses a 512-byte BufReader (hook requests are small JSON, ~100-200 bytes).
fn handle_conn(stream: UnixStream, _dir: &Path) {
    // 100ms timeout to avoid blocking the listener thread
    stream.set_read_timeout(Some(std::time::Duration::from_millis(100))).ok();
    stream.set_write_timeout(Some(std::time::Duration::from_millis(100))).ok();

    let mut reader = BufReader::with_capacity(512, &stream);
    let mut line = String::with_capacity(256);
    if reader.read_line(&mut line).is_err() { return; }
    let line = line.trim();
    if line.is_empty() { return; }

    // Fast-path: extract "op" without full JSON parse for the common case
    let op = crate::hook::extract_json_str(line, "op").unwrap_or("");
    let result = match op {
        "search" => {
            let req = match crate::json::parse(line) { Ok(v) => v, Err(_) => return };
            handle_search(&req)
        }
        "topics" => handle_topics(),
        "ambient" => handle_ambient_fast(line),
        _ => String::new(),
    };

    let mut writer = stream;
    let _ = writer.write_all(result.as_bytes());
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}

/// Search the in-memory index.
/// Request: {"op":"search","query":"cache","limit":5}
fn handle_search(req: &crate::json::Value) -> String {
    let query = req.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = req.get("limit").and_then(|v| v.as_f64()).unwrap_or(5.0) as usize;
    crate::mcp::with_index(|data| {
        crate::binquery::search(data, query, limit).unwrap_or_default()
    }).unwrap_or_default()
}

/// Return topic table from in-memory index.
/// Request: {"op":"topics"}
fn handle_topics() -> String {
    crate::mcp::with_index(|data| {
        let topics = crate::binquery::topic_table(data).unwrap_or_default();
        let mut list: Vec<String> = topics.iter()
            .map(|(_, name, count)| format!("{name} ({count})"))
            .collect();
        list.sort();
        list.join(", ")
    }).unwrap_or_default()
}

/// Combined ambient hook query with fast string extraction — no full JSON parse needed.
/// Request: {"op":"ambient","stem":"cache","syms":["removed1","removed2"]}
fn handle_ambient_fast(line: &str) -> String {
    let stem = match crate::hook::extract_json_str(line, "stem") {
        Some(s) if !s.is_empty() => s,
        _ => return String::new(),
    };

    // Extract syms array manually: find "syms":[ then parse quoted strings
    let syms = extract_syms_array(line);

    crate::mcp::with_index(|data| {
        query_ambient_core(data, stem, &syms)
    }).unwrap_or_default()
}

/// Shared ambient query logic used by both socket and disk-fallback paths.
fn query_ambient_core(data: &[u8], stem: &str, syms: &[&str]) -> String {
    let results = crate::binquery::search(data, stem, 5).unwrap_or_default();
    let has_results = !results.is_empty() && !results.starts_with("0 match");

    let sq = format!("structural {stem}");
    let structural = crate::binquery::search(data, &sq, 3).unwrap_or_default();
    let has_structural = !structural.is_empty() && !structural.starts_with("0 match");

    let mut refactor = String::new();
    if !syms.is_empty() {
        let sym_list = syms.join(", ");
        refactor.push_str(&format!("\nREFACTOR IMPACT (symbols modified: {sym_list}):\n"));
        for sym in syms {
            let hits = crate::binquery::search(data, sym, 3).unwrap_or_default();
            if !hits.is_empty() && !hits.starts_with("0 match") {
                refactor.push_str(&hits);
            }
        }
    }
    let has_refactor = !refactor.is_empty();
    if !has_results && !has_structural && !has_refactor { return String::new(); }

    let mut out = String::new();
    if has_results { out.push_str(&format!("amaranthine entries for {stem}:\n{results}")); }
    if has_structural {
        if has_results { out.push_str("\n---\n"); }
        out.push_str(&format!("structural coupling:\n{structural}"));
    }
    if has_refactor {
        if has_results || has_structural { out.push_str("\n---\n"); }
        out.push_str(&refactor);
    }
    out
}

/// Extract string array from "syms":["a","b","c"] without full JSON parse.
fn extract_syms_array(line: &str) -> Vec<&str> {
    let needle = "\"syms\":[";
    let pos = match line.find(needle) {
        Some(p) => p + needle.len(),
        None => return Vec::new(),
    };
    let rest = &line[pos..];
    let end = match rest.find(']') {
        Some(e) => e,
        None => return Vec::new(),
    };
    let arr = &rest[..end];
    arr.split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1) // odd positions are inside quotes
        .map(|(_, s)| s)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Client: query the running MCP server's socket. Returns None if unavailable.
/// Uses small BufReader (512 bytes) — responses are typically under 1KB.
pub fn query(dir: &Path, request: &str) -> Option<String> {
    let path = sock_path(dir);
    let mut stream = UnixStream::connect(&path).ok()?;
    stream.set_read_timeout(Some(std::time::Duration::from_millis(50))).ok();
    stream.set_write_timeout(Some(std::time::Duration::from_millis(50))).ok();
    stream.write_all(request.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    stream.flush().ok()?;
    let mut reader = BufReader::with_capacity(1024, stream);
    let mut response = String::with_capacity(512);
    reader.read_line(&mut response).ok()?;
    let trimmed = response.trim();
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}
