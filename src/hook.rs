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
/// v7.3: Smart Ambient Context — multi-layer search with source-path matching,
/// symbol extraction from file, topic affinity, and deduplication.
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
    let file_path = extract_json_str(input, "file_path")
        .or_else(|| extract_json_str(input, "\"path\""))
        .unwrap_or("");
    if file_path.is_empty() { return Ok(String::new()); }

    let stem = std::path::Path::new(file_path)
        .file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.len() < 3 { return Ok(String::new()); }

    // Extract removed symbols for Edit refactor detection (needs full parse)
    let syms = if is_edit {
        match crate::json::parse(input) {
            Ok(val) => extract_removed_syms(&val, stem),
            Err(_) => vec![],
        }
    } else { vec![] };

    let data = match mmap_index(dir) {
        Some(d) => d,
        None => return Ok(String::new()),
    };
    let sym_refs: Vec<&str> = syms.iter().map(|s| s.as_str()).collect();
    let out = query_ambient(data, stem, file_path, &sym_refs);
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

/// Smart Ambient Context: multi-layer search with deduplication.
/// v7.3: source-path matching, symbol extraction from file, topic affinity.
/// Layer 1: Source-path matches (entries with [source: ...filename...])
/// Layer 2: Symbol-based OR search (symbols extracted from file content)
/// Layer 3: Global BM25 search (existing stem search, capped)
/// Layer 4: Structural coupling (existing)
/// Layer 5: Refactor impact (existing, Edit only)
pub fn query_ambient(data: &[u8], stem: &str, file_path: &str, syms: &[&str]) -> String {
    let filename = std::path::Path::new(file_path)
        .file_name().and_then(|f| f.to_str()).unwrap_or(stem);
    let mut seen = crate::fxhash::FxHashSet::default();
    let mut out = String::new();

    // Layer 1: Source-path matches — entries explicitly about this file
    let source_ids = crate::binquery::source_entries_for_file(data, filename).unwrap_or_default();
    if !source_ids.is_empty() {
        out.push_str("source-linked (");
        out.push_str(filename);
        out.push_str("):\n");
        for &eid in &source_ids {
            seen.insert(eid);
            if let Ok(snip) = crate::binquery::entry_snippet(data, eid) {
                if !snip.is_empty() {
                    out.push_str("  ");
                    out.push_str(&snip);
                    out.push('\n');
                }
            }
        }
    }

    // Layer 2: Symbol-based search — extract fn/struct/enum names from file, OR-search
    let file_symbols = extract_file_symbols(file_path);
    if !file_symbols.is_empty() {
        let query = build_symbol_query(&file_symbols, stem);
        if !query.is_empty() {
            let filter = crate::binquery::FilterPred::none();
            let hits = crate::binquery::search_v2_or(data, &query, &filter, 8)
                .unwrap_or_default();
            let new_hits: Vec<_> = hits.into_iter()
                .filter(|h| seen.insert(h.entry_id))
                .take(5)
                .collect();
            if !new_hits.is_empty() {
                if !out.is_empty() { out.push_str("---\n"); }
                out.push_str("symbol context:\n");
                for hit in &new_hits {
                    out.push_str("  ");
                    out.push_str(&hit.snippet);
                    out.push('\n');
                }
            }
        }
    }

    // Layer 3: Global BM25 search (stem keyword)
    let global = crate::binquery::search_v2(data, stem, 5).unwrap_or_default();
    let global_new: Vec<_> = global.into_iter()
        .filter(|h| seen.insert(h.entry_id))
        .take(3)
        .collect();
    if !global_new.is_empty() {
        if !out.is_empty() { out.push_str("---\n"); }
        out.push_str("related (");
        out.push_str(stem);
        out.push_str("):\n");
        for hit in &global_new {
            out.push_str("  ");
            out.push_str(&hit.snippet);
            out.push('\n');
        }
    }

    // Layer 4: Structural coupling
    let mut sq_buf = [0u8; 128];
    let sq_prefix = b"structural ";
    let sq_len = sq_prefix.len() + stem.len();
    let structural = if sq_len <= sq_buf.len() {
        sq_buf[..sq_prefix.len()].copy_from_slice(sq_prefix);
        sq_buf[sq_prefix.len()..sq_len].copy_from_slice(stem.as_bytes());
        let sq = unsafe { std::str::from_utf8_unchecked(&sq_buf[..sq_len]) };
        crate::binquery::search_v2(data, sq, 3).unwrap_or_default()
    } else {
        let mut sq = String::with_capacity(sq_len);
        sq.push_str("structural ");
        sq.push_str(stem);
        crate::binquery::search_v2(data, &sq, 3).unwrap_or_default()
    };
    let structural_new: Vec<_> = structural.into_iter()
        .filter(|h| seen.insert(h.entry_id))
        .collect();
    if !structural_new.is_empty() {
        if !out.is_empty() { out.push_str("---\n"); }
        out.push_str("structural coupling:\n");
        for hit in &structural_new {
            out.push_str("  ");
            out.push_str(&hit.snippet);
            out.push('\n');
        }
    }

    // Layer 5: Refactor impact (Edit only)
    if !syms.is_empty() {
        let mut refactor = String::new();
        refactor.push_str("REFACTOR IMPACT (symbols modified: ");
        for (i, sym) in syms.iter().enumerate() {
            if i > 0 { refactor.push_str(", "); }
            refactor.push_str(sym);
        }
        refactor.push_str("):\n");
        let mut has_hits = false;
        for sym in syms {
            let hits = crate::binquery::search_v2(data, sym, 3).unwrap_or_default();
            for hit in hits {
                if seen.insert(hit.entry_id) {
                    refactor.push_str("  ");
                    refactor.push_str(&hit.snippet);
                    refactor.push('\n');
                    has_hits = true;
                }
            }
        }
        if has_hits {
            if !out.is_empty() { out.push_str("---\n"); }
            out.push_str(&refactor);
        }
    }

    out
}

/// Extract key symbol names (fn/struct/enum/trait/class) from a source file.
/// Reads the file directly — hook has filesystem access.
/// Returns raw symbol names for tokenization into search terms.
/// Caps at 500 lines and 20 symbols to bound cost.
fn extract_file_symbols(path: &str) -> Vec<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    static KEYWORDS: &[&str] = &[
        "fn ", "struct ", "enum ", "trait ",              // Rust
        "func ", "class ", "protocol ", "extension ",     // Swift
    ];

    let mut symbols = Vec::with_capacity(16);
    for line in content.lines().take(500) {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("///")
            || trimmed.starts_with('#') || trimmed.starts_with("/*") { continue; }
        for kw in KEYWORDS {
            if let Some(pos) = trimmed.find(kw) {
                let rest = &trimmed[pos + kw.len()..];
                // Skip generic params: impl<T> Foo → start after Foo
                let rest = if *kw == "fn " || *kw == "func " {
                    rest
                } else {
                    rest.trim_start_matches(|c: char| c == '<' || c == '\'')
                        .split(|c: char| c == '>' || c == ' ')
                        .next().unwrap_or(rest)
                };
                let name: String = rest.chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.len() >= 3 && name.as_bytes()[0].is_ascii_alphabetic() {
                    symbols.push(name);
                }
            }
        }
    }
    symbols.sort();
    symbols.dedup();
    symbols.truncate(20);
    symbols
}

/// Build a search query from extracted symbols.
/// Uses compound forms (CamelCase joined) for specificity.
/// Excludes the stem to avoid redundancy with Layer 3.
fn build_symbol_query(symbols: &[String], stem: &str) -> String {
    let mut terms = Vec::with_capacity(symbols.len());
    let stem_lower = stem.to_lowercase();
    for sym in symbols {
        // Tokenize to get compound forms + components
        let tokens = crate::text::tokenize(sym);
        for tok in tokens {
            if tok.len() >= 3 && tok != stem_lower {
                terms.push(tok);
            }
        }
    }
    terms.sort();
    terms.dedup();
    terms.truncate(15); // cap query terms
    terms.join(" ")
}

/// PermissionRequest: auto-approve all amaranthine MCP tool calls.
/// Static constant — zero allocations.
const APPROVE_MCP_RESPONSE: &str =
    r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;
