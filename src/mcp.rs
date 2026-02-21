mod dispatch;
mod tools;

pub use dispatch::dispatch;

use crate::json::Value;
use std::io::{self, BufRead, Write as _};
use std::path::Path;
use std::sync::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};

static SESSION_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct ServerIndex { data: Vec<u8> }

static INDEX: RwLock<Option<ServerIndex>> = RwLock::new(None);
static INDEX_DIRTY: AtomicBool = AtomicBool::new(false);
static DIRTY_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

pub(crate) fn log_session(msg: String) {
    if let Ok(mut log) = SESSION_LOG.lock() { log.push(msg); }
}

pub fn run(dir: &Path) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    ensure_datalog(dir);
    recover_index(dir);

    // Start Unix socket listener for hook queries against in-memory index
    let _sock_guard = crate::sock::start_listener(dir);

    if std::env::var("AMARANTHINE_REEXEC").is_ok() {
        std::env::remove_var("AMARANTHINE_REEXEC");
        // Self-audit: store binary UUID + git hash for crash correlation
        if let Some(audit) = build_audit_entry() {
            let _ = crate::store::run_full_ext(dir, "amaranthine-audit", &audit,
                Some("system,reload"), true, None, None, None);
            after_write(dir, "amaranthine-audit");
        }
        let mut out = stdout.lock();
        let _ = writeln!(out, r#"{{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}}"#);
        let _ = out.flush();
    }

    // Reusable line buffer — avoids allocation per message
    let mut line_buf = String::with_capacity(4096);
    let stdin_lock = stdin.lock();
    let mut reader = io::BufReader::new(stdin_lock);

    loop {
        line_buf.clear();
        match reader.read_line(&mut line_buf) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => { return Err(e.to_string()); }
        }
        let line = line_buf.trim();
        if line.is_empty() || line.len() > 10_000_000 { continue; }
        let msg = match crate::json::parse(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id");

        // All branches write directly to stdout and continue — zero intermediate String.
        match method {
            "initialize" => {
                let id_json = id_to_json(id);
                let mut out = stdout.lock();
                let _ = write!(out,
                    r#"{{"jsonrpc":"2.0","id":{id_json},"result":{INIT_RESULT}}}"#);
                let _ = writeln!(out);
                let _ = out.flush();
            }
            "notifications/initialized" | "initialized" => {}
            "tools/list" => {
                let id_json = id_to_json(id);
                let tools_json = tools::tool_list_json();
                let mut out = stdout.lock();
                let _ = write!(out, r#"{{"jsonrpc":"2.0","id":{id_json},"result":{tools_json}}}"#);
                let _ = writeln!(out);
                let _ = out.flush();
            }
            "tools/call" => {
                let p = msg.get("params");
                let name = p.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                let id_json = id_to_json(id);
                if name == "_reload" {
                    let mut out = stdout.lock();
                    let _ = writeln!(out,
                        r#"{{"jsonrpc":"2.0","id":{id_json},"result":{{"content":[{{"type":"text","text":"reloading amaranthine..."}}]}}}}"#);
                    let _ = out.flush();
                    drop(out);
                    do_reload();
                    continue;
                }
                let args = p.and_then(|p| p.get("arguments"));
                let mut out = stdout.lock();
                let ok = match dispatch::dispatch(name, args, dir) {
                    Ok(ref text) => write_rpc_ok(&mut out, &id_json, text),
                    Err(ref e) => write_rpc_err(&mut out, &id_json, e),
                };
                if let Err(e) = ok {
                    eprintln!("amaranthine: stdout write error: {e}");
                    break;
                }
                let _ = out.flush();
            }
            "ping" => {
                let id_json = id_to_json(id);
                let mut out = stdout.lock();
                let _ = write!(out, r#"{{"jsonrpc":"2.0","id":{id_json},"result":{{}}}}"#);
                let _ = writeln!(out);
                let _ = out.flush();
            }
            _ if id.is_some() => {
                let id_json = id_to_json(id);
                let mut out = stdout.lock();
                let _ = write!(out, r#"{{"jsonrpc":"2.0","id":{id_json},"error":{{"code":-32601,"message":"method not found"}}}}"#);
                let _ = writeln!(out);
                let _ = out.flush();
            }
            _ => {}
        }
    }
    Ok(())
}

/// Write id Value to stack buffer — zero heap allocation for the 99% case (integer IDs).
/// Returns a small stack string that derefs to &str.
fn id_to_json(id: Option<&Value>) -> IdBuf {
    match id {
        Some(Value::Num(n)) if n.fract() == 0.0 => {
            let mut buf = IdBuf { bytes: [0u8; 24], len: 0 };
            let v = *n as i64;
            if v == 0 { buf.bytes[0] = b'0'; buf.len = 1; return buf; }
            let (neg, mut uv) = if v < 0 { (true, (-(v as i128)) as u64) } else { (false, v as u64) };
            let mut i = 24;
            while uv > 0 { i -= 1; buf.bytes[i] = b'0' + (uv % 10) as u8; uv /= 10; }
            if neg { i -= 1; buf.bytes[i] = b'-'; }
            buf.bytes.copy_within(i..24, 0);
            buf.len = (24 - i) as u8;
            buf
        }
        Some(v) => {
            let s = v.to_string();
            let mut buf = IdBuf { bytes: [0u8; 24], len: s.len().min(24) as u8 };
            buf.bytes[..buf.len as usize].copy_from_slice(&s.as_bytes()[..buf.len as usize]);
            buf
        }
        None => {
            let mut buf = IdBuf { bytes: [0u8; 24], len: 4 };
            buf.bytes[..4].copy_from_slice(b"null");
            buf
        }
    }
}

/// Stack-allocated ID buffer — avoids heap allocation for JSON-RPC id formatting.
/// MCP IDs are almost always small integers (1-999), fitting easily in 24 bytes.
struct IdBuf { bytes: [u8; 24], len: u8 }
impl std::ops::Deref for IdBuf {
    type Target = str;
    fn deref(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }
}
impl std::fmt::Display for IdBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(self)
    }
}

/// Streaming JSON-RPC success response — writes directly to stdout, zero intermediate String.
/// This is the hot path for every tools/call response.
fn write_rpc_ok(w: &mut impl io::Write, id_json: &str, text: &str) -> io::Result<()> {
    w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":")?;
    w.write_all(id_json.as_bytes())?;
    w.write_all(b",\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"")?;
    write_json_escaped(w, text)?;
    w.write_all(b"\"}]}}\n")
}

/// Streaming JSON-RPC error response — writes directly to stdout, zero intermediate String.
fn write_rpc_err(w: &mut impl io::Write, id_json: &str, msg: &str) -> io::Result<()> {
    w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":")?;
    w.write_all(id_json.as_bytes())?;
    w.write_all(b",\"error\":{\"code\":-32603,\"message\":\"")?;
    write_json_escaped(w, msg)?;
    w.write_all(b"\"}}\n")
}

/// Write JSON-escaped string directly to a writer (no intermediate String allocation).
/// Byte-level chunk-copy: scans for escape-needing bytes, writes clean chunks via write_all.
fn write_json_escaped(w: &mut impl io::Write, s: &str) -> io::Result<()> {
    let bytes = s.as_bytes();
    let mut last_copy = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let esc: &[u8] = match b {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            c if c < 0x20 => {
                if last_copy < i { w.write_all(&bytes[last_copy..i])?; }
                write!(w, "\\u{:04x}", c)?;
                last_copy = i + 1;
                continue;
            }
            _ => continue,
        };
        if last_copy < i { w.write_all(&bytes[last_copy..i])?; }
        w.write_all(esc)?;
        last_copy = i + 1;
    }
    if last_copy < bytes.len() { w.write_all(&bytes[last_copy..])?; }
    Ok(())
}


fn do_reload() {
    use std::os::unix::process::CommandExt;
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let src = exe.parent()
        .and_then(|p| p.parent())
        .and_then(|_| {
            let manifest = std::env::var("AMARANTHINE_SRC").ok()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    let home = std::env::var("HOME").unwrap_or_default();
                    std::path::PathBuf::from(home).join("wudan/dojo/crash3/amaranthine")
                });
            let release = manifest.join("target/release/amaranthine");
            if release.exists() { Some(release) } else { None }
        });
    if let Some(src_bin) = src {
        // Atomic copy: write to temp file, then rename (prevents corrupted binary on crash)
        let tmp = exe.with_extension("tmp");
        if let Err(e) = std::fs::copy(&src_bin, &tmp) {
            eprintln!("reload: copy failed: {e}");
        } else if let Err(e) = std::fs::rename(&tmp, &exe) {
            eprintln!("reload: rename failed: {e}");
            let _ = std::fs::remove_file(&tmp);
        } else {
            let _ = std::process::Command::new("codesign")
                .args(["-s", "-", "-f"]).arg(&exe).output();
        }
    }
    std::env::set_var("AMARANTHINE_REEXEC", "1");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _err = std::process::Command::new(&exe).args(&args).exec();
    std::env::remove_var("AMARANTHINE_REEXEC");
    eprintln!("reload failed: {_err}");
}

fn ensure_datalog(dir: &Path) {
    if !crate::config::data_log_exists(dir) {
        if let Ok(files) = crate::config::list_topic_files(dir) {
            if !files.is_empty() {
                match crate::datalog::migrate_from_md(dir) {
                    Ok(n) => {
                        eprintln!("amaranthine: migrated {n} entries from .md to data.log");
                        for path in &files { let _ = std::fs::remove_file(path); }
                    }
                    Err(e) => eprintln!("amaranthine: migration failed: {e}"),
                }
            } else { let _ = crate::datalog::ensure_log(dir); }
        } else { let _ = crate::datalog::ensure_log(dir); }
    }
    match crate::inverted::rebuild_and_persist(dir) {
        Ok((_, bytes)) => store_index(bytes),
        Err(_) => {} // no index yet, load_index in run() will try disk
    }
}

/// Validate existing index.bin; if corrupted or wrong version, rebuild from data.log.
/// Called on startup before first query, and on any index read failure.
pub(crate) fn recover_index(dir: &Path) {
    let index_path = dir.join("index.bin");
    let needs_rebuild = match std::fs::read(&index_path) {
        Ok(data) => crate::binquery::read_header(&data).is_err(),
        Err(_) => true,
    };
    if needs_rebuild {
        eprintln!("amaranthine: index.bin invalid, rebuilding from data.log...");
        match crate::inverted::rebuild_and_persist(dir) {
            Ok((msg, bytes)) => {
                eprintln!("amaranthine: {}", msg.lines().next().unwrap_or("rebuilt"));
                store_index(bytes);
            }
            Err(e) => eprintln!("amaranthine: rebuild failed: {e}"),
        }
    }
}

pub(crate) fn load_index(dir: &Path) {
    let index_path = dir.join("index.bin");
    if let Ok(data) = std::fs::read(&index_path) {
        store_index(data);
    }
}

pub(crate) fn store_index(data: Vec<u8>) {
    if let Ok(mut guard) = INDEX.write() {
        *guard = Some(ServerIndex { data });
    }
}

/// Borrow cached index data via closure. Returns None if no index loaded.
/// Uses RwLock read guard — concurrent reads don't block each other.
pub(crate) fn with_index<F, R>(f: F) -> Option<R>
where F: FnOnce(&[u8]) -> R {
    INDEX.read().ok().and_then(|guard| guard.as_ref().map(|idx| f(&idx.data)))
}

pub(crate) fn after_write(_dir: &Path, _topic: &str) {
    INDEX_DIRTY.store(true, Ordering::Release);
    // Record when dirty flag was set for debounce
    if let Ok(mut guard) = DIRTY_AT.lock() {
        if guard.is_none() { *guard = Some(std::time::Instant::now()); }
    }
}

/// Rebuild index if dirty and debounce window (50ms) has elapsed.
/// Burst writes within the window are coalesced into a single rebuild.
/// Persists index.bin to disk (atomic rename) so hook mmap always reads fresh data.
/// v6.6: single DIRTY_AT lock acquisition (was two: check + clear).
pub(crate) fn ensure_index_fresh(dir: &Path) {
    if !INDEX_DIRTY.load(Ordering::Acquire) { return; }
    // Single lock: check debounce AND clear in one acquisition
    let should_rebuild = DIRTY_AT.lock().ok().map_or(false, |mut g| {
        match *g {
            Some(t) if t.elapsed() < std::time::Duration::from_millis(50) => false,
            _ => {
                // Only clear if we win the CAS
                if INDEX_DIRTY.compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                    *g = None;
                    true
                } else { false }
            }
        }
    });
    if should_rebuild {
        match crate::inverted::rebuild(dir) {
            Ok((_, bytes)) => {
                let tmp = dir.join("index.bin.tmp");
                let target = dir.join("index.bin");
                let _ = std::fs::write(&tmp, &bytes)
                    .and_then(|_| std::fs::rename(&tmp, &target));
                store_index(bytes);
            }
            Err(_) => load_index(dir),
        }
    }
}

/// Pre-serialized initialize result — zero allocation, written directly to stdout.
const INIT_RESULT: &str = r#"{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"amaranthine","version":"10.0.0"}}"#;

/// Build audit entry with binary UUID and git hash for crash correlation.
fn build_audit_entry() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let data = std::fs::read(&exe).ok()?;
    let uuid = parse_macho_uuid(&data)?;
    let hash = read_git_hash().unwrap_or_default();
    if hash.is_empty() {
        Some(format!("_reload v8.0.0 UUID:{uuid}"))
    } else {
        Some(format!("_reload v8.0.0 UUID:{uuid} git:{hash}"))
    }
}

/// Parse Mach-O binary to extract LC_UUID load command.
/// Reads the 64-bit Mach-O header, iterates load commands to find cmd=0x1B (LC_UUID).
fn parse_macho_uuid(data: &[u8]) -> Option<String> {
    if data.len() < 32 { return None; }
    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    // MH_MAGIC_64 = 0xFEEDFACF
    if magic != 0xFEEDFACF { return None; }
    let ncmds = u32::from_le_bytes([data[16], data[17], data[18], data[19]]) as usize;
    // Mach-O 64 header size = 32 bytes
    let mut off = 32;
    for _ in 0..ncmds {
        if off + 8 > data.len() { break; }
        let cmd = u32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]]);
        let cmdsize = u32::from_le_bytes([data[off+4], data[off+5], data[off+6], data[off+7]]) as usize;
        if cmdsize < 8 { break; }
        // LC_UUID = 0x1B, payload is 16 bytes at offset+8
        if cmd == 0x1B && off + 24 <= data.len() {
            let uuid = &data[off+8..off+24];
            let mut s = String::with_capacity(36);
            for (i, b) in uuid.iter().enumerate() {
                if i == 4 || i == 6 || i == 8 || i == 10 { s.push('-'); }
                s.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                s.push(char::from(b"0123456789ABCDEF"[(b & 0xF) as usize]));
            }
            return Some(s);
        }
        off += cmdsize;
    }
    None
}

/// Read current git commit hash from the source repo.
fn read_git_hash() -> Option<String> {
    let src = std::env::var("AMARANTHINE_SRC").ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            std::path::PathBuf::from(home).join("wudan/dojo/crash3/amaranthine")
        });
    let head = std::fs::read_to_string(src.join(".git/HEAD")).ok()?;
    let head = head.trim();
    if let Some(ref_path) = head.strip_prefix("ref: ") {
        let hash = std::fs::read_to_string(src.join(".git").join(ref_path)).ok()?;
        Some(hash.trim().chars().take(12).collect())
    } else {
        Some(head.chars().take(12).collect())
    }
}
