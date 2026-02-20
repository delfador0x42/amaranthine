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

/// Build hook JSON output with direct string formatting — zero Value allocations.
/// JSON-escapes the context string inline.
fn hook_output(context: &str) -> String {
    let mut out = String::with_capacity(64 + context.len());
    out.push_str(r#"{"hookSpecificOutput":{"additionalContext":""#);
    json_escape_into(context.as_bytes(), &mut out);
    out.push_str(r#""}}"#);
    out
}

/// Escape a string for JSON embedding (no surrounding quotes).
/// Public for use by mcp.rs response formatting.
/// Handles UTF-8 correctly by working on &str and escaping only JSON-special chars.
pub fn json_escape_into(s: &[u8], buf: &mut String) {
    // Fast path: scan for chars that need escaping
    let mut i = 0;
    let mut last_copy = 0;
    while i < s.len() {
        let c = s[i];
        let escape = match c {
            b'"' => Some("\\\""),
            b'\\' => Some("\\\\"),
            b'\n' => Some("\\n"),
            b'\r' => Some("\\r"),
            b'\t' => Some("\\t"),
            c if c < 0x20 => None, // handled below
            _ => { i += 1; continue; }
        };
        // Copy everything before this char
        if last_copy < i {
            // Safety: input is valid UTF-8 (came from &str), and we only break at ASCII
            if let Ok(chunk) = std::str::from_utf8(&s[last_copy..i]) {
                buf.push_str(chunk);
            }
        }
        if let Some(esc) = escape {
            buf.push_str(esc);
        } else {
            use std::fmt::Write;
            let _ = write!(buf, "\\u{:04x}", c);
        }
        i += 1;
        last_copy = i;
    }
    // Copy trailing chunk
    if last_copy < s.len() {
        if let Ok(chunk) = std::str::from_utf8(&s[last_copy..]) {
            buf.push_str(chunk);
        }
    }
}

/// PreToolUse: inject amaranthine entries relevant to the file being accessed.
/// Uses fast-path byte scanning to extract tool_name and file_path without full JSON parse.
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

    // Fast path: query MCP server via Unix socket (in-memory index)
    if let Some(result) = sock_ambient(dir, stem, &syms) {
        if result.is_empty() { return Ok(String::new()); }
        return Ok(hook_output(&result));
    }

    // Fallback: read index.bin from disk
    let data = match std::fs::read(dir.join("index.bin")) {
        Ok(d) => d,
        Err(_) => return Ok(String::new()),
    };
    let out = query_ambient(&data, stem, &syms);
    if out.is_empty() { return Ok(String::new()); }
    Ok(hook_output(&out))
}

/// Fast JSON string extraction: find "key":"value" without full parse.
/// Returns the unescaped value or None if not found.
/// Works for simple string values (no nested escapes needed for our keys).
/// Public for use by sock.rs.
pub fn extract_json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    // Search for "key":" pattern
    let needle = if key.starts_with('"') {
        // Already quoted (for ambiguous keys like "path")
        format!("{}:\"", key)
    } else {
        format!("\"{}\":\"", key)
    };
    let pos = json.find(&needle)?;
    let val_start = pos + needle.len();
    // Find closing quote (handle escaped quotes)
    let rest = &json[val_start..];
    let mut end = 0;
    let bytes = rest.as_bytes();
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
fn subagent_start(dir: &Path) -> Result<String, String> {
    let fallback = "AMARANTHINE KNOWLEDGE STORE: You have access to amaranthine MCP tools. \
         Search before starting work.";

    // Fast path: query MCP server via socket
    let topic_list = crate::sock::query(dir, r#"{"op":"topics"}"#)
        .or_else(|| {
            // Fallback: read index.bin from disk
            let data = std::fs::read(dir.join("index.bin")).ok()?;
            let topics = crate::binquery::topic_table(&data).ok()?;
            let mut list: Vec<String> = topics.iter()
                .map(|(_, name, count)| format!("{name} ({count})"))
                .collect();
            list.sort();
            Some(list.join(", "))
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

/// Try ambient query via Unix socket (MCP server's in-memory index).
fn sock_ambient(dir: &Path, stem: &str, syms: &[String]) -> Option<String> {
    let syms_json: Vec<String> = syms.iter().map(|s| format!("\"{s}\"")).collect();
    let req = format!(
        r#"{{"op":"ambient","stem":"{stem}","syms":[{syms}]}}"#,
        syms = syms_json.join(",")
    );
    crate::sock::query(dir, &req)
}

/// Extract symbols removed by an Edit (for refactor impact detection).
fn extract_removed_syms(input: &crate::json::Value, stem: &str) -> Vec<String> {
    let ti = input.get("tool_input");
    let old = ti.and_then(|t| t.get("old_string")).and_then(|v| v.as_str()).unwrap_or("");
    let new_str = ti.and_then(|t| t.get("new_string")).and_then(|v| v.as_str()).unwrap_or("");
    if old.len() < 8 { return vec![]; }
    let extract = |s: &str| -> std::collections::HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|w| w.len() >= 4 && w.bytes().any(|b| b.is_ascii_alphabetic()))
            .map(|w| w.to_lowercase())
            .collect()
    };
    let old_tokens: std::collections::HashSet<String> = extract(old)
        .into_iter().filter(|t| t != stem).collect();
    let new_tokens: std::collections::HashSet<String> = extract(new_str);
    let mut removed: Vec<String> = old_tokens.into_iter()
        .filter(|t| !new_tokens.contains(t))
        .collect();
    removed.sort();
    removed.truncate(3);
    removed
}

/// Run ambient queries against index data (disk fallback path).
fn query_ambient(data: &[u8], stem: &str, syms: &[String]) -> String {
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

/// PermissionRequest: auto-approve all amaranthine MCP tool calls.
/// Static constant — zero allocations.
const APPROVE_MCP_RESPONSE: &str =
    r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;
