//! Claude Code hook handlers: ambient context, build reminders, session management.
//!
//! Performance: all hooks use direct string formatting — zero Value tree allocations.
//! Hook output is JSON, but we build it with format!() not Value::Obj().to_string().

use std::io::Read;
use std::path::Path;

pub fn run(hook_type: &str, dir: &Path) -> Result<String, String> {
    // approve-mcp and stop need no stdin at all
    match hook_type {
        "approve-mcp" => return Ok(APPROVE_MCP_RESPONSE.into()),
        "stop" => return stop(),
        _ => {}
    }

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok();
    let input = input.trim();

    match hook_type {
        "ambient" => ambient(input, dir),
        "post-build" => post_build(input),
        "subagent-start" => subagent_start(dir),
        _ => Err(format!("unknown hook type: {hook_type}")),
    }
}

/// Memory-map index.bin for zero-copy queries — no socket overhead, no full file read.
/// Uses mmap(2) directly — zero external dependencies.
/// Returns None if file doesn't exist or is too small.
/// Mapping lives until process exit (no munmap needed for short-lived hook processes).
fn mmap_index(dir: &Path) -> Option<&'static [u8]> {
    let path = dir.join("index.bin");
    let f = std::fs::File::open(&path).ok()?;
    let len = f.metadata().ok()?.len() as usize;
    if len < std::mem::size_of::<crate::format::Header>() { return None; }

    use std::os::unix::io::AsRawFd;
    let fd = f.as_raw_fd();

    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    }

    let ptr = unsafe { mmap(std::ptr::null_mut(), len, 1 /* PROT_READ */, 2 /* MAP_PRIVATE */, fd, 0) };
    drop(f); // close fd — mapping persists

    if ptr.is_null() || ptr as usize == usize::MAX { return None; } // MAP_FAILED
    Some(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Build hook JSON output with direct string formatting — zero Value allocations.
/// JSON-escapes the context string inline via json::escape_into.
/// Public for use by sock.rs hook relay handler.
pub fn hook_output(context: &str) -> String {
    let mut out = String::with_capacity(64 + context.len());
    out.push_str(r#"{"hookSpecificOutput":{"additionalContext":""#);
    crate::json::escape_into(context, &mut out);
    out.push_str(r#""}}"#);
    out
}

/// PreToolUse: inject amaranthine entries relevant to the file being accessed.
/// Uses fast-path byte scanning to extract tool_name and file_path without full JSON parse.
/// v6.5: mmap index.bin directly — eliminates socket round-trip (~150-300μs saved per hook).
fn ambient(input: &str, dir: &Path) -> Result<String, String> {
    if input.is_empty() { return Ok(String::new()); }

    // Fast-path: extract tool_name via byte scan (avoid full JSON parse)
    let tool = extract_json_str(input, "tool_name").unwrap_or("");
    let is_edit = tool == "Edit";
    match tool {
        "Read" | "Edit" | "Write" | "Glob" | "Grep" | "NotebookEdit" => {}
        _ => return Ok(String::new()),
    }

    // Fast-path: extract file_path or path from tool_input
    let path = extract_json_str(input, "file_path")
        .or_else(|| extract_json_str(input, "\"path\""))
        .unwrap_or("");
    if path.is_empty() { return Ok(String::new()); }

    let stem = std::path::Path::new(path)
        .file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.len() < 3 { return Ok(String::new()); }

    // Extract removed symbols for Edit refactor detection (needs full parse)
    let syms = if is_edit {
        // Only parse the full JSON if we actually need old_string/new_string
        match crate::json::parse(input) {
            Ok(val) => extract_removed_syms(&val, stem),
            Err(_) => vec![],
        }
    } else { vec![] };

    // Fast path: mmap index.bin — zero-copy, no socket overhead, no full file read.
    // MCP server persists index.bin on every hot rebuild (atomic rename).
    let data = match mmap_index(dir) {
        Some(d) => d,
        None => return Ok(String::new()),
    };
    let sym_refs: Vec<&str> = syms.iter().map(|s| s.as_str()).collect();
    let out = query_ambient(data, stem, &sym_refs);
    if out.is_empty() { return Ok(String::new()); }
    Ok(hook_output(&out))
}

/// Fast JSON string extraction: find "key":"value" without full parse.
/// Returns the unescaped value or None if not found.
/// Works for simple string values (no nested escapes needed for our keys).
/// Uses stack-allocated needle — zero heap allocation.
/// Public for use by sock.rs.
pub fn extract_json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    // Build needle on stack: "key":" or key:" (if already quoted)
    let kb = key.as_bytes();
    let quoted = kb.first() == Some(&b'"');
    let mut needle_buf = [0u8; 80];
    let nlen = if quoted {
        if kb.len() + 2 > needle_buf.len() { return None; }
        needle_buf[..kb.len()].copy_from_slice(kb);
        needle_buf[kb.len()] = b':';
        needle_buf[kb.len() + 1] = b'"';
        kb.len() + 2
    } else {
        if kb.len() + 4 > needle_buf.len() { return None; }
        needle_buf[0] = b'"';
        needle_buf[1..1 + kb.len()].copy_from_slice(kb);
        needle_buf[1 + kb.len()] = b'"';
        needle_buf[2 + kb.len()] = b':';
        needle_buf[3 + kb.len()] = b'"';
        kb.len() + 4
    };
    // Safety: needle is ASCII (keys are ASCII identifiers)
    let needle = unsafe { std::str::from_utf8_unchecked(&needle_buf[..nlen]) };
    let pos = json.find(needle)?;
    let val_start = pos + nlen;
    // Find closing quote (handle escaped quotes)
    let rest = &json[val_start..];
    let bytes = rest.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        if bytes[end] == b'"' && (end == 0 || bytes[end - 1] != b'\\') {
            return Some(&rest[..end]);
        }
        end += 1;
    }
    None
}

/// PostToolUse(Bash, async): after build commands, remind to store results.
/// Uses fast byte scan to detect build commands, then direct string output.
fn post_build(input: &str) -> Result<String, String> {
    // Fast-path: scan for build keywords in raw JSON without parsing
    let is_build = (input.contains("xcodebuild") && input.contains("build"))
        || input.contains("cargo build") || input.contains("swift build")
        || input.contains("swiftc ");
    if !is_build { return Ok(String::new()); }
    // Static response — no Value allocation needed
    Ok(POST_BUILD_RESPONSE.into())
}

const POST_BUILD_RESPONSE: &str = r#"{"systemMessage":"BUILD COMPLETED. If the build failed with a non-obvious error, store the root cause in amaranthine (topic: build-gotchas). If it succeeded after fixing an issue, store what fixed it."}"#;

/// Stop: remind to store findings before conversation ends.
fn stop() -> Result<String, String> {
    let stamp = "/tmp/amaranthine-hook-stop.last";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(content) = std::fs::read_to_string(stamp) {
        if let Ok(last) = content.trim().parse::<u64>() {
            if now.saturating_sub(last) < 120 { return Ok(String::new()); }
        }
    }
    std::fs::write(stamp, now.to_string()).ok();
    Ok(hook_output("STOPPING: Store any non-obvious findings in amaranthine before ending."))
}

/// SubagentStart: inject dynamic topic list from index.
/// v6.5: prefer mmap over socket — eliminates connect overhead.
fn subagent_start(dir: &Path) -> Result<String, String> {
    let fallback = "AMARANTHINE KNOWLEDGE STORE: You have access to amaranthine MCP tools. \
         Search before starting work.";

    // Fast path: mmap index.bin directly
    let topic_list = mmap_index(dir)
        .and_then(|data| {
            let topics = crate::binquery::topic_table(data).ok()?;
            let mut list: Vec<String> = topics.iter()
                .map(|(_, name, count)| format!("{name} ({count})"))
                .collect();
            list.sort();
            Some(list.join(", "))
        })
        .or_else(|| {
            // Fallback: socket query to running MCP server
            crate::sock::query(dir, r#"{"op":"topics"}"#)
        });

    let msg = match topic_list {
        Some(list) if !list.is_empty() => format!(
            "AMARANTHINE KNOWLEDGE STORE: You have access to amaranthine MCP tools. \
             BEFORE starting work, call mcp__amaranthine__search with keywords \
             relevant to your task. Topics: {list}"),
        _ => fallback.into(),
    };
    Ok(hook_output(&msg))
}

/// Extract symbols removed by an Edit (for refactor impact detection).
/// Public for use by sock.rs hook relay handler.
/// v6.6: FxHashSet (~3ns/op) replaces std::HashSet (SipHash ~20ns/op).
pub fn extract_removed_syms(input: &crate::json::Value, stem: &str) -> Vec<String> {
    let ti = input.get("tool_input");
    let old = ti.and_then(|t| t.get("old_string")).and_then(|v| v.as_str()).unwrap_or("");
    let new_str = ti.and_then(|t| t.get("new_string")).and_then(|v| v.as_str()).unwrap_or("");
    if old.len() < 8 { return vec![]; }
    let extract = |s: &str| -> crate::fxhash::FxHashSet<String> {
        s.split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|w| w.len() >= 4 && w.bytes().any(|b| b.is_ascii_alphabetic()))
            .map(|w| w.to_lowercase())
            .collect()
    };
    let old_tokens: crate::fxhash::FxHashSet<String> = extract(old)
        .into_iter().filter(|t| t != stem).collect();
    let new_tokens: crate::fxhash::FxHashSet<String> = extract(new_str);
    let mut removed: Vec<String> = old_tokens.into_iter()
        .filter(|t| !new_tokens.contains(t))
        .collect();
    removed.sort();
    removed.truncate(3);
    removed
}

/// Run ambient queries against index data (mmap or disk).
/// Zero format!() calls — all output built with push_str.
/// v6.6: unified implementation used by both hook.rs and sock.rs — takes &[&str] for syms.
/// v6.5: stack-allocated structural query — zero heap alloc for query string.
pub fn query_ambient(data: &[u8], stem: &str, syms: &[&str]) -> String {
    let results = crate::binquery::search(data, stem, 5).unwrap_or_default();
    let has_results = !results.is_empty() && !results.starts_with("0 match");

    // Stack-allocated query string for structural search
    let mut sq_buf = [0u8; 128];
    let sq_prefix = b"structural ";
    let sq_len = sq_prefix.len() + stem.len();
    let structural = if sq_len <= sq_buf.len() {
        sq_buf[..sq_prefix.len()].copy_from_slice(sq_prefix);
        sq_buf[sq_prefix.len()..sq_len].copy_from_slice(stem.as_bytes());
        let sq = unsafe { std::str::from_utf8_unchecked(&sq_buf[..sq_len]) };
        crate::binquery::search(data, sq, 3).unwrap_or_default()
    } else {
        let mut sq = String::with_capacity(sq_len);
        sq.push_str("structural ");
        sq.push_str(stem);
        crate::binquery::search(data, &sq, 3).unwrap_or_default()
    };
    let has_structural = !structural.is_empty() && !structural.starts_with("0 match");

    let mut refactor = String::new();
    if !syms.is_empty() {
        refactor.push_str("\nREFACTOR IMPACT (symbols modified: ");
        for (i, sym) in syms.iter().enumerate() {
            if i > 0 { refactor.push_str(", "); }
            refactor.push_str(sym);
        }
        refactor.push_str("):\n");
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
    if has_results {
        out.push_str("amaranthine entries for ");
        out.push_str(stem);
        out.push_str(":\n");
        out.push_str(&results);
    }
    if has_structural {
        if has_results { out.push_str("\n---\n"); }
        out.push_str("structural coupling:\n");
        out.push_str(&structural);
    }
    if has_refactor {
        if has_results || has_structural { out.push_str("\n---\n"); }
        out.push_str(&refactor);
    }
    out
}

/// PermissionRequest: auto-approve all amaranthine MCP tool calls.
/// Static constant — zero allocations.
const APPROVE_MCP_RESPONSE: &str =
    r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;
