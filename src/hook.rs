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
        "stop" => return stop(dir),
        _ => {}
    }

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok();
    let input = input.trim();

    match hook_type {
        "ambient" => ambient(input, dir),
        "post-build" => post_build(input, dir),
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
/// v10: Session-aware — dedup via injected set, track files + tools.
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

    // Load session for dedup + tracking
    let mut session = crate::session::Session::load_or_new(dir);
    session.record_tool(tool);

    // Track file operation
    let file_op = match tool {
        "Write" => crate::session::FileOp::Created,
        "Edit" | "NotebookEdit" => crate::session::FileOp::Edited,
        _ => crate::session::FileOp::Read,
    };
    session.track_file(file_path, file_op);

    // Extract removed symbols for Edit refactor detection (needs full parse)
    let syms = if is_edit {
        match crate::json::parse(input) {
            Ok(val) => extract_removed_syms(&val, stem),
            Err(_) => vec![],
        }
    } else { vec![] };

    let data = match mmap_index(dir) {
        Some(d) => d,
        None => {
            session.save(dir).ok();
            return Ok(String::new());
        }
    };
    let sym_refs: Vec<&str> = syms.iter().map(|s| s.as_str()).collect();
    let out = query_ambient(data, stem, file_path, &sym_refs, Some(&mut session));

    // Save session (writes dedup state + file tracking)
    session.save(dir).ok();

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

/// PostToolUse(Bash, async): after build commands, track build state in session.
/// v10: Only remind on failure — successful builds are silent.
fn post_build(input: &str, dir: &Path) -> Result<String, String> {
    let is_build = (input.contains("xcodebuild") && input.contains("build"))
        || input.contains("cargo build") || input.contains("swift build")
        || input.contains("swiftc ");
    if !is_build { return Ok(String::new()); }

    // Detect build failure vs success from output
    let has_error = input.contains("error:") || input.contains("BUILD FAILED")
        || input.contains("error[E") || input.contains("error: could not compile");
    let has_success = input.contains("Build Succeeded") || input.contains("Finished")
        || input.contains("BUILD SUCCEEDED") || input.contains("Compiling ");

    // Extract first few error lines for session state
    let mut errors = Vec::new();
    if has_error {
        // Find "stdout" or "stderr" field content and extract error lines
        for line in input.lines() {
            let trimmed = line.trim();
            if (trimmed.contains("error:") || trimmed.contains("error[E"))
                && !trimmed.contains("generated") && errors.len() < 5
            {
                // Clean up the error line (strip JSON escaping artifacts)
                let clean = trimmed.replace("\\n", "").replace("\\\"", "\"");
                if clean.len() > 10 && clean.len() < 300 {
                    errors.push(clean);
                }
            }
        }
    }

    let build_ok = !has_error || (has_success && !has_error);

    // Update session with build state
    let mut session = crate::session::Session::load_or_new(dir);
    session.record_build(build_ok, errors);
    session.record_tool("Bash");
    session.save(dir).ok();

    // Only remind on failure — successful builds are quiet
    if build_ok {
        Ok(String::new())
    } else {
        Ok(POST_BUILD_FAIL_RESPONSE.into())
    }
}

const POST_BUILD_FAIL_RESPONSE: &str = r#"{"systemMessage":"BUILD FAILED. Store the root cause in amaranthine (topic: build-gotchas) if the error was non-obvious. Check session state for extracted errors."}"#;

/// Stop: flush pending notes from session, remind to store findings.
/// v10: Session-aware — includes session summary in stop message.
fn stop(dir: &Path) -> Result<String, String> {
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

    // Load session for summary
    let session = crate::session::Session::load(dir);
    let mut msg = String::with_capacity(256);
    msg.push_str("STOPPING: Store any non-obvious findings in amaranthine before ending.");

    if let Some(s) = &session {
        let duration_min = now.saturating_sub(s.started) / 60;
        let files_edited = s.files.iter()
            .filter(|f| matches!(f.op, crate::session::FileOp::Edited | crate::session::FileOp::Created))
            .count();
        let entries_injected = s.injected.len();

        {
            msg.push_str(" Session: ");
            push_u64_str(&mut msg, duration_min);
            msg.push_str("min, ");
            push_u64_str(&mut msg, files_edited as u64);
            msg.push_str(" files changed, ");
            push_u64_str(&mut msg, entries_injected as u64);
            msg.push_str(" entries injected, phase=");
            msg.push_str(s.phase.as_str());
            msg.push('.');
        }

        if !s.pending_notes.is_empty() {
            msg.push_str(" PENDING NOTES TO STORE: ");
            for (i, note) in s.pending_notes.iter().enumerate() {
                if i > 0 { msg.push_str("; "); }
                msg.push_str(note);
            }
        }
    }

    Ok(hook_output(&msg))
}

fn push_u64_str(buf: &mut String, n: u64) {
    use std::fmt::Write;
    write!(buf, "{n}").unwrap();
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

/// Smart Ambient Context: multi-layer search with cross-invocation deduplication.
/// v10.1: Unified function — Option<Session> for session dedup + auto-focus topics.
///
/// Layers (each deduplicates against all prior via FxHashSet<u32>):
///   1. Source-path matches — entries with [source:] metadata for this file
///   2. Symbol-based OR search — fn/struct/enum names extracted from file
///   3. Global BM25 search — stem keyword
///   4. Structural coupling — "structural <stem>" query
///   5. Refactor impact — removed symbols (Edit only)
///
/// When session=Some: skips entries already injected this session, marks new ones,
/// and auto-infers focus topics from entry topic names (3+ hits threshold).
pub fn query_ambient(
    data: &[u8], stem: &str, file_path: &str, syms: &[&str],
    session: Option<&mut crate::session::Session>,
) -> String {
    let filename = std::path::Path::new(file_path)
        .file_name().and_then(|f| f.to_str()).unwrap_or(stem);
    let mut seen = crate::fxhash::FxHashSet::default();
    let mut entry_ids: Vec<u32> = Vec::with_capacity(32);
    let mut snippet_pool: Vec<std::borrow::Cow<str>> = Vec::with_capacity(32);

    // Snapshot session injected set for dedup (immutable borrow)
    let injected_snapshot: Option<crate::fxhash::FxHashSet<u32>> = session.as_ref()
        .map(|s| s.injected.clone());

    // Dedup: local seen set + session injected (if available)
    let mut check_add = |eid: u32| -> bool {
        if let Some(ref inj) = injected_snapshot {
            if inj.contains(&eid) { return false; }
        }
        seen.insert(eid)
    };

    // Layer 1: Source-path matches
    let source_ids = crate::binquery::source_entries_for_file(data, filename).unwrap_or_default();
    let l1_start = snippet_pool.len();
    for &eid in &source_ids {
        if check_add(eid) {
            if let Ok(snip) = crate::binquery::entry_snippet_ref(data, eid) {
                if !snip.is_empty() {
                    snippet_pool.push(std::borrow::Cow::Borrowed(snip));
                    entry_ids.push(eid);
                }
            }
        }
    }
    let l1_count = snippet_pool.len() - l1_start;

    // Layer 2: Symbol-based search — skip if Layer 1 already provided enough context.
    let l2_start = snippet_pool.len();
    if source_ids.len() < 5 {
        let file_symbols = cached_file_symbols(file_path);
        if !file_symbols.is_empty() {
            let query = build_symbol_query(&file_symbols, stem);
            if !query.is_empty() {
                let filter = crate::binquery::FilterPred::none();
                let hits = crate::binquery::search_v2_or(data, &query, &filter, 8)
                    .unwrap_or_default();
                for h in hits {
                    if check_add(h.entry_id) {
                        snippet_pool.push(std::borrow::Cow::Owned(h.snippet));
                        entry_ids.push(h.entry_id);
                        if snippet_pool.len() - l2_start >= 5 { break; }
                    }
                }
            }
        }
    }
    let l2_count = snippet_pool.len() - l2_start;

    // Layer 3: Global BM25 search (stem keyword)
    let l3_start = snippet_pool.len();
    let global = crate::binquery::search_v2(data, stem, 5).unwrap_or_default();
    for h in global {
        if check_add(h.entry_id) {
            snippet_pool.push(std::borrow::Cow::Owned(h.snippet));
            entry_ids.push(h.entry_id);
            if snippet_pool.len() - l3_start >= 3 { break; }
        }
    }
    let l3_count = snippet_pool.len() - l3_start;

    // Layer 4: Structural coupling
    let l4_start = snippet_pool.len();
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
    for h in structural {
        if check_add(h.entry_id) {
            snippet_pool.push(std::borrow::Cow::Owned(h.snippet));
            entry_ids.push(h.entry_id);
        }
    }
    let l4_count = snippet_pool.len() - l4_start;

    // Layer 5: Refactor impact (Edit only)
    let l5_start = snippet_pool.len();
    if !syms.is_empty() {
        for sym in syms {
            let hits = crate::binquery::search_v2(data, sym, 3).unwrap_or_default();
            for hit in hits {
                if check_add(hit.entry_id) {
                    snippet_pool.push(std::borrow::Cow::Owned(hit.snippet));
                    entry_ids.push(hit.entry_id);
                }
            }
        }
    }
    let l5_count = snippet_pool.len() - l5_start;

    if snippet_pool.is_empty() { return String::new(); }

    // Session bookkeeping: mark injected + auto-infer focus topics
    drop(check_add);
    if let Some(session) = session {
        for &eid in &entry_ids {
            session.mark_injected(eid);
        }
        // Auto-infer focus topics: count hits per topic, add topics with 3+ hits
        let mut topic_counts: crate::fxhash::FxHashMap<u16, u16> = crate::fxhash::map_with_capacity(8);
        for &eid in &entry_ids {
            if let Ok(tid) = crate::binquery::entry_topic_id(data, eid) {
                *topic_counts.entry(tid).or_insert(0) += 1;
            }
        }
        for (&tid, &count) in &topic_counts {
            if count >= 3 {
                if let Ok(name) = crate::binquery::topic_name(data, tid) {
                    session.add_focus_topic(&name);
                }
            }
        }
    }

    // Single output pass
    let est_cap = snippet_pool.iter().map(|s| s.len() + 4).sum::<usize>() + 5 * 40;
    let mut out = String::with_capacity(est_cap);

    let counts = [l1_count, l2_count, l3_count, l4_count, l5_count];
    let labels = ["source-linked", "symbol context", "related", "structural coupling", "REFACTOR IMPACT"];
    let mut pool_idx = 0;
    for (i, &count) in counts.iter().enumerate() {
        if count == 0 { continue; }
        if !out.is_empty() { out.push_str("---\n"); }

        match i {
            0 => { out.push_str("source-linked ("); out.push_str(filename); out.push_str("):\n"); }
            2 => { out.push_str("related ("); out.push_str(stem); out.push_str("):\n"); }
            4 => {
                out.push_str("REFACTOR IMPACT (symbols modified: ");
                for (j, sym) in syms.iter().enumerate() {
                    if j > 0 { out.push_str(", "); }
                    out.push_str(sym);
                }
                out.push_str("):\n");
            }
            _ => { out.push_str(labels[i]); out.push_str(":\n"); }
        }

        for _ in 0..count {
            out.push_str("  ");
            out.push_str(&snippet_pool[pool_idx]);
            out.push('\n');
            pool_idx += 1;
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

/// 1-entry LRU symbol cache: filesystem-based, persists across hook invocations.
/// Cache hit avoids file read + parse (~0.8ms savings per invocation).
/// Keyed on (path, mtime_secs) — auto-invalidates when file is modified.
const SYM_CACHE_PATH: &str = "/tmp/amr-sym-cache";

fn cached_file_symbols(path: &str) -> Vec<String> {
    let mtime = match std::fs::metadata(path) {
        Ok(m) => m.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs()).unwrap_or(0),
        Err(_) => return vec![],
    };

    // Cache hit: path + mtime match → return cached symbols
    if let Ok(cache) = std::fs::read_to_string(SYM_CACHE_PATH) {
        let mut lines = cache.lines();
        if let (Some(cp), Some(cm)) = (lines.next(), lines.next()) {
            if cp == path {
                if let Ok(cached_mt) = cm.parse::<u64>() {
                    if cached_mt == mtime {
                        return lines.map(|l| l.to_string()).collect();
                    }
                }
            }
        }
    }

    // Cache miss: extract, write cache, return
    let syms = extract_file_symbols(path);
    let mut buf = String::with_capacity(path.len() + 32 + syms.len() * 20);
    buf.push_str(path);
    buf.push('\n');
    itoa_push_u64(&mut buf, mtime);
    for sym in &syms {
        buf.push('\n');
        buf.push_str(sym);
    }
    std::fs::write(SYM_CACHE_PATH, buf.as_bytes()).ok();
    syms
}

fn itoa_push_u64(buf: &mut String, n: u64) {
    if n == 0 { buf.push('0'); return; }
    let mut digits = [0u8; 20];
    let mut i = 0;
    let mut v = n;
    while v > 0 { digits[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; buf.push(digits[i] as char); }
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
