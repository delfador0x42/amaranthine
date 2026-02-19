use crate::json::Value;
use std::io::{self, BufRead, Write as _};
use std::path::Path;
use std::sync::Mutex;

/// Session log: one-line summaries of stores this session.
static SESSION_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// In-memory binary index — loaded on startup, rebuilt after writes.
/// Eliminates fs::read on every index_search call.
struct ServerIndex {
    data: Vec<u8>,
    state: crate::binquery::QueryState,
}

static INDEX: Mutex<Option<ServerIndex>> = Mutex::new(None);

fn log_session(msg: String) {
    if let Ok(mut log) = SESSION_LOG.lock() {
        log.push(msg);
    }
}

pub fn run(dir: &Path) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    // Pre-warm cache + load binary index on startup
    let _ = warm_cache(dir);
    load_index(dir);

    // After re-exec, notify client that tools may have changed
    if std::env::var("AMARANTHINE_REEXEC").is_ok() {
        std::env::remove_var("AMARANTHINE_REEXEC");
        let notif = Value::Obj(vec![
            ("jsonrpc".into(), Value::Str("2.0".into())),
            ("method".into(), Value::Str("notifications/tools/list_changed".into())),
        ]);
        let mut out = stdout.lock();
        let _ = writeln!(out, "{notif}");
        let _ = out.flush();
    }

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.is_empty() { continue; }
        let msg = match crate::json::parse(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id");

        // Handle _reload specially — must exec after responding
        if method == "tools/call" {
            let p = msg.get("params");
            let name = p.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("");
            if name == "_reload" {
                let resp = rpc_ok(id, content_result("reloading amaranthine..."));
                let mut out = stdout.lock();
                let _ = writeln!(out, "{resp}");
                let _ = out.flush();
                drop(out);
                do_reload();
                continue;
            }
        }

        let resp = match method {
            "initialize" => Some(rpc_ok(id, init_result())),
            "notifications/initialized" | "initialized" => None,
            "tools/list" => Some(rpc_ok(id, Value::Obj(vec![
                ("tools".into(), tool_list()),
            ]))),
            "tools/call" => {
                let p = msg.get("params");
                let name = p.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                let args = p.and_then(|p| p.get("arguments"));
                Some(match dispatch(name, args, dir) {
                    Ok(text) => rpc_ok(id, content_result(&text)),
                    Err(e) => rpc_err(id, -32603, &e),
                })
            }
            "ping" => Some(rpc_ok(id, Value::Obj(Vec::new()))),
            _ => id.map(|_| rpc_err(id, -32601, "method not found")),
        };

        if let Some(r) = resp {
            let mut out = stdout.lock();
            if let Err(e) = writeln!(out, "{r}") {
                eprintln!("amaranthine: stdout write error: {e}");
                break;
            }
            let _ = out.flush();
        }
    }
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
        if let Err(e) = std::fs::copy(&src_bin, &exe) {
            eprintln!("reload: copy failed: {e}");
        } else {
            let _ = std::process::Command::new("codesign")
                .args(["-s", "-", "-f"])
                .arg(&exe)
                .output();
        }
    }

    std::env::set_var("AMARANTHINE_REEXEC", "1");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _err = std::process::Command::new(&exe).args(&args).exec();
    std::env::remove_var("AMARANTHINE_REEXEC");
    eprintln!("reload failed: {_err}");
}

fn init_result() -> Value {
    Value::Obj(vec![
        ("protocolVersion".into(), Value::Str("2024-11-05".into())),
        ("capabilities".into(), Value::Obj(vec![
            ("tools".into(), Value::Obj(Vec::new())),
        ])),
        ("serverInfo".into(), Value::Obj(vec![
            ("name".into(), Value::Str("amaranthine".into())),
            ("version".into(), Value::Str("2.0.0".into())),
        ])),
    ])
}

fn rpc_ok(id: Option<&Value>, result: Value) -> Value {
    Value::Obj(vec![
        ("jsonrpc".into(), Value::Str("2.0".into())),
        ("id".into(), id.cloned().unwrap_or(Value::Null)),
        ("result".into(), result),
    ])
}

fn rpc_err(id: Option<&Value>, code: i64, msg: &str) -> Value {
    Value::Obj(vec![
        ("jsonrpc".into(), Value::Str("2.0".into())),
        ("id".into(), id.cloned().unwrap_or(Value::Null)),
        ("error".into(), Value::Obj(vec![
            ("code".into(), Value::Num(code)),
            ("message".into(), Value::Str(msg.into())),
        ])),
    ])
}

fn content_result(text: &str) -> Value {
    Value::Obj(vec![("content".into(), Value::Arr(vec![
        Value::Obj(vec![
            ("type".into(), Value::Str("text".into())),
            ("text".into(), Value::Str(text.into())),
        ]),
    ]))])
}

fn tool(name: &str, desc: &str, req: &[&str], props: &[(&str, &str, &str)]) -> Value {
    Value::Obj(vec![
        ("name".into(), Value::Str(name.into())),
        ("description".into(), Value::Str(desc.into())),
        ("inputSchema".into(), Value::Obj(vec![
            ("type".into(), Value::Str("object".into())),
            ("properties".into(), Value::Obj(props.iter().map(|(n, t, d)| {
                ((*n).into(), Value::Obj(vec![
                    ("type".into(), Value::Str((*t).into())),
                    ("description".into(), Value::Str((*d).into())),
                ]))
            }).collect())),
            ("required".into(), Value::Arr(
                req.iter().map(|r| Value::Str((*r).into())).collect()
            )),
        ])),
    ])
}

/// Build a batch_store tool definition with proper array-of-objects schema.
fn batch_store_tool() -> Value {
    let entry_schema = Value::Obj(vec![
        ("type".into(), Value::Str("object".into())),
        ("properties".into(), Value::Obj(vec![
            ("topic".into(), Value::Obj(vec![
                ("type".into(), Value::Str("string".into())),
                ("description".into(), Value::Str("Topic name".into())),
            ])),
            ("text".into(), Value::Obj(vec![
                ("type".into(), Value::Str("string".into())),
                ("description".into(), Value::Str("Entry content".into())),
            ])),
            ("tags".into(), Value::Obj(vec![
                ("type".into(), Value::Str("string".into())),
                ("description".into(), Value::Str("Comma-separated tags".into())),
            ])),
        ])),
        ("required".into(), Value::Arr(vec![
            Value::Str("topic".into()), Value::Str("text".into()),
        ])),
    ]);

    Value::Obj(vec![
        ("name".into(), Value::Str("batch_store".into())),
        ("description".into(), Value::Str(
            "Store multiple entries in one call. Each entry: {topic, text, tags?}. Faster than sequential store calls.".into()
        )),
        ("inputSchema".into(), Value::Obj(vec![
            ("type".into(), Value::Str("object".into())),
            ("properties".into(), Value::Obj(vec![
                ("entries".into(), Value::Obj(vec![
                    ("type".into(), Value::Str("array".into())),
                    ("items".into(), entry_schema),
                    ("description".into(), Value::Str("Array of entries to store".into())),
                ])),
                ("verbose".into(), Value::Obj(vec![
                    ("type".into(), Value::Str("string".into())),
                    ("description".into(), Value::Str(
                        "Set to 'true' for per-entry details (default: terse count only)".into()
                    )),
                ])),
            ])),
            ("required".into(), Value::Arr(vec![Value::Str("entries".into())])),
        ])),
    ])
}

/// Shared search filter properties for tool definitions.
const SEARCH_FILTER_PROPS: &[(&str, &str, &str)] = &[
    ("limit", "string", "Max results to return (default: unlimited)"),
    ("after", "string", "Only entries on/after date (YYYY-MM-DD or 'today'/'yesterday'/'this-week')"),
    ("before", "string", "Only entries on/before date (YYYY-MM-DD or 'today'/'yesterday')"),
    ("tag", "string", "Only entries with this tag"),
    ("topic", "string", "Limit search to a single topic"),
    ("mode", "string", "Search mode: 'and' (default, all terms must match) or 'or' (any term matches)"),
];

fn tool_list() -> Value {
    // Build search props: query + detail + shared filter props
    let search_props: Vec<(&str, &str, &str)> = [
        ("query", "string", "Search query"),
        ("detail", "string", "Result detail level: 'full', 'medium' (default), or 'brief'"),
    ].into_iter()
        .chain(SEARCH_FILTER_PROPS.iter().copied())
        .collect();

    // Count-only doesn't need limit or detail
    let search_count_props: Vec<(&str, &str, &str)> = std::iter::once(("query", "string", "Search query"))
        .chain(SEARCH_FILTER_PROPS.iter().copied().filter(|(n, _, _)| *n != "limit"))
        .collect();

    Value::Arr(vec![
        tool("store", "Store a timestamped knowledge entry under a topic. Warns on duplicate content.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Entry content"),
              ("tags", "string", "Comma-separated tags (e.g. 'bug,p0,iris')"),
              ("force", "string", "Set to 'true' to bypass duplicate detection"),
              ("terse", "string", "Set to 'true' for minimal response (just first line)")]),
        tool("append", "Add text to the last entry in a topic (no new timestamp). Use when adding related info to a recent entry.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Text to append")]),
        batch_store_tool(),
        tool("search", "Search all knowledge files (case-insensitive). Splits CamelCase/snake_case. Falls back to OR when AND finds nothing.",
            &[], &search_props),
        tool("search_brief", "Quick search: just topic names + first matching line per hit",
            &[], &search_props),
        tool("search_medium", "Medium search: topic + timestamp + first 2 content lines per hit. Between brief and full.",
            &[], &search_props),
        tool("search_count", "Count matching sections without returning content. Fast way to gauge query scope.",
            &[], &search_count_props),
        tool("search_topics", "Show which topics matched and how many hits per topic. Best first step before deep search.",
            &[], &search_count_props),
        tool("context", "Session briefing: topics + recent entries (7 days) + optional search",
            &[],
            &[("query", "string", "Optional search query"),
              ("brief", "string", "Set to 'true' for compact mode (topics only, no recent)")]),
        tool("topics", "List all topic files with entry and line counts",
            &[], &[]),
        tool("recent", "Show entries from last N days (or hours) across all topics",
            &[],
            &[("days", "string", "Number of days (default: 7)"),
              ("hours", "string", "Number of hours (overrides days for finer granularity)")]),
        tool("delete_entry", "Remove the most recent entry from a topic",
            &["topic"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Delete entry matching this substring instead of last"),
              ("index", "string", "Delete entry by index number (from list_entries)")]),
        tool("delete_topic", "Delete an entire topic and all its entries",
            &["topic"],
            &[("topic", "string", "Topic name")]),
        tool("append_entry", "Add text to an existing entry found by substring match, index, or tag (keeps timestamp, preserves body)",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Substring to find the entry to append to"),
              ("index", "string", "Entry index number (from list_entries)"),
              ("tag", "string", "Append to most recent entry with this tag"),
              ("text", "string", "Text to append to the entry")]),
        tool("update_entry", "Overwrite an existing entry's text (keeps timestamp). Adds [modified] marker.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Substring to find the entry to update"),
              ("index", "string", "Entry index number (from list_entries)"),
              ("text", "string", "Replacement text for the entry")]),
        tool("read_topic", "Read the full contents of a specific topic file",
            &["topic"],
            &[("topic", "string", "Topic name")]),
        tool("digest", "Compact summary of all topics (one bullet per entry)",
            &[], &[]),
        tool("list_tags", "List all tags used across all topics with counts",
            &[], &[]),
        tool("stats", "Show stats: topic count, entry count, date range, tag count",
            &[], &[]),
        tool("list_entries", "List entries in a topic with index numbers. For bulk review before delete.",
            &["topic"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Only show entries matching this substring")]),
        tool("prune", "Flag stale topics (no entries in N days). For identifying outdated knowledge.",
            &[],
            &[("days", "string", "Stale threshold in days (default: 30)")]),
        tool("compact", "Find and merge duplicate entries within a topic. Without topic, scans all topics.",
            &[],
            &[("topic", "string", "Topic to compact (omit to scan all)"),
              ("apply", "string", "Set to 'true' to actually merge (default: dry run)")]),
        tool("export", "Export all topics as structured JSON for backup or migration.",
            &[], &[]),
        tool("import", "Import topics from JSON (merges with existing data).",
            &["json"],
            &[("json", "string", "JSON string to import")]),
        tool("xref", "Find cross-references: entries in other topics that mention this topic.",
            &["topic"],
            &[("topic", "string", "Topic to find references for")]),
        tool("migrate", "Find and fix entries without proper timestamps.",
            &[],
            &[("apply", "string", "Set to 'true' to backfill timestamps (default: dry run)")]),
        tool("get_entry", "Fetch a single entry by topic and index number. Use after list_entries to read specific entries.",
            &["topic", "index"],
            &[("topic", "string", "Topic name"),
              ("index", "string", "Entry index number (0-based, from list_entries)")]),
        tool("rename_topic", "Rename a topic (moves the file). All entries preserved.",
            &["topic", "new_name"],
            &[("topic", "string", "Current topic name"),
              ("new_name", "string", "New topic name")]),
        tool("tag_entry", "Add or remove tags on an existing entry. Use to mark entries as superseded or add missing tags.",
            &["topic", "tags"],
            &[("topic", "string", "Topic name"),
              ("index", "string", "Entry index number (from list_entries)"),
              ("match_str", "string", "Substring to find the entry"),
              ("tags", "string", "Comma-separated tags to add"),
              ("remove", "string", "Comma-separated tags to remove")]),
        tool("rebuild_index", "Rebuild the binary inverted index from all topic files. Enables fast index_search.",
            &[], &[]),
        tool("index_stats", "Show binary index and cache statistics.",
            &[], &[]),
        tool("index_search", "Search using the binary inverted index (~200ns per query). Requires rebuild_index first.",
            &["query"],
            &[("query", "string", "Search query"),
              ("limit", "string", "Max results (default: 10)")]),
        tool("session", "Show what was stored this session. Tracks all store/batch_store calls since server started.",
            &[], &[]),
        tool("_reload", "Re-exec the server binary to pick up code changes. Sends tools/list_changed notification after reload.",
            &[], &[]),
    ])
}

fn build_filter(args: Option<&Value>) -> crate::search::Filter {
    let after = resolve_date_shortcut(&arg_str(args, "after"));
    let before = resolve_date_shortcut(&arg_str(args, "before"));
    let tag = arg_str(args, "tag");
    let topic = arg_str(args, "topic");
    let mode = match arg_str(args, "mode").as_str() {
        "or" => crate::search::SearchMode::Or,
        _ => crate::search::SearchMode::And,
    };
    crate::search::Filter {
        after: if after.is_empty() { None } else { crate::time::parse_date_days(&after) },
        before: if before.is_empty() { None } else { crate::time::parse_date_days(&before) },
        tag: if tag.is_empty() { None } else { Some(tag) },
        topic: if topic.is_empty() { None } else { Some(topic) },
        mode,
    }
}

/// Resolve date shortcuts to YYYY-MM-DD strings.
fn resolve_date_shortcut(s: &str) -> String {
    let now = crate::time::LocalTime::now();
    match s {
        "today" => format!("{:04}-{:02}-{:02}", now.year, now.month, now.day),
        "yesterday" => {
            let d = now.to_days() - 1;
            days_to_date(d)
        }
        "this-week" | "this_week" | "week" => {
            let d = now.to_days() - 7;
            days_to_date(d)
        }
        "this-month" | "this_month" | "month" => {
            let d = now.to_days() - 30;
            days_to_date(d)
        }
        _ => s.to_string(),
    }
}

/// Convert days-since-epoch back to YYYY-MM-DD.
fn days_to_date(z: i64) -> String {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn warm_cache(dir: &Path) -> Result<(), String> {
    let files = crate::config::list_search_files(dir)?;
    crate::cache::with_corpus(&files, |_| {});
    Ok(())
}

/// Load binary index into memory. Called on startup and after writes.
fn load_index(dir: &Path) {
    let index_path = dir.join("index.bin");
    if let Ok(data) = std::fs::read(&index_path) {
        let n = crate::binquery::entry_count(&data).unwrap_or(0);
        let state = crate::binquery::QueryState::new(n);
        if let Ok(mut guard) = INDEX.lock() {
            *guard = Some(ServerIndex { data, state });
        }
    }
}

/// Invalidate cache for a topic, rebuild + reload binary index.
fn after_write(dir: &Path, topic: &str) {
    let f = crate::config::sanitize_topic(topic);
    crate::cache::invalidate(&dir.join(format!("{f}.md")));
    let _ = crate::inverted::rebuild(dir);
    load_index(dir);
}

pub fn dispatch(name: &str, args: Option<&Value>, dir: &Path) -> Result<String, String> {
    match name {
        "store" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            let tags = arg_str(args, "tags");
            let tags = if tags.is_empty() { None } else { Some(tags.as_str()) };
            let force = arg_bool(args, "force");
            let terse = arg_bool(args, "terse");
            let result = crate::store::run_full(dir, &topic, &text, tags, force)?;
            after_write(dir, &topic);
            log_session(format!("[{}] {}", topic,
                result.lines().next().unwrap_or("stored")));
            if terse {
                Ok(result.lines().next().unwrap_or(&result).to_string())
            } else {
                Ok(result)
            }
        }
        "append" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            let result = crate::store::append(dir, &topic, &text)?;
            after_write(dir, &topic);
            Ok(result)
        }
        "batch_store" => {
            let verbose = arg_bool(args, "verbose");
            let items = args.and_then(|a| a.get("entries"))
                .and_then(|v| match v { Value::Arr(a) => Some(a), _ => None })
                .ok_or("entries must be an array")?;
            if items.len() > 30 {
                return Err(format!(
                    "batch too large ({} entries, max 30). Split into smaller batch_store calls.",
                    items.len()
                ));
            }
            // Single lock for entire batch
            let _lock = crate::lock::FileLock::acquire(dir)?;
            let mut ok_count = 0;
            let mut results = Vec::new();
            let mut seen: Vec<(String, String)> = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let topic = item.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let tags = item.get("tags").and_then(|v| v.as_str());
                if topic.is_empty() || text.is_empty() {
                    results.push(format!("  [{}] skipped: missing topic or text", i + 1));
                    continue;
                }
                let key = (
                    topic.to_lowercase(),
                    text.chars().take(60).collect::<String>().to_lowercase(),
                );
                if seen.iter().any(|s| s.0 == key.0 && s.1 == key.1) {
                    results.push(format!("  [{}] skipped: duplicate within batch", i + 1));
                    continue;
                }
                seen.push(key);
                match crate::store::run_batch_entry(dir, topic, text, tags) {
                    Ok(msg) => {
                        ok_count += 1;
                        let f = crate::config::sanitize_topic(topic);
                        crate::cache::invalidate(&dir.join(format!("{f}.md")));
                        let first = msg.lines().next().unwrap_or(&msg);
                        results.push(format!("  [{}] {}", i + 1, first));
                        log_session(format!("[{}] {}", topic, first));
                    }
                    Err(e) => {
                        let first = e.lines().next().unwrap_or(&e);
                        results.push(format!("  [{}] err: {}", i + 1, first));
                    }
                }
            }
            drop(_lock);
            if ok_count > 0 {
                let _ = crate::inverted::rebuild(dir);
                load_index(dir);
            }
            if verbose {
                Ok(format!("batch: {ok_count}/{} stored\n{}", items.len(), results.join("\n")))
            } else {
                Ok(format!("batch: {ok_count}/{} stored", items.len()))
            }
        }
        "search" => {
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().ok();
            let detail = arg_str(args, "detail");
            let filter = build_filter(args);
            match detail.as_str() {
                "full" => crate::search::run(dir, &query, true, limit, &filter),
                "brief" => crate::search::run_brief(dir, &query, limit, &filter),
                _ => crate::search::run_medium(dir, &query, limit, &filter),
            }
        }
        "search_brief" => {
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().ok();
            let filter = build_filter(args);
            crate::search::run_brief(dir, &query, limit, &filter)
        }
        "search_medium" => {
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().ok();
            let filter = build_filter(args);
            crate::search::run_medium(dir, &query, limit, &filter)
        }
        "search_count" => {
            let query = arg_str(args, "query");
            let filter = build_filter(args);
            crate::search::count(dir, &query, &filter)
        }
        "search_topics" => {
            let query = arg_str(args, "query");
            let filter = build_filter(args);
            crate::search::run_topics(dir, &query, &filter)
        }
        "context" => {
            let q = arg_str(args, "query");
            let q = if q.is_empty() { None } else { Some(q.as_str()) };
            let brief = arg_str(args, "brief");
            if brief == "true" {
                crate::context::run_brief(dir, q, true)
            } else {
                crate::context::run(dir, q, true)
            }
        }
        "topics" => crate::topics::list(dir),
        "recent" => {
            let h = arg_str(args, "hours");
            if let Ok(hours) = h.parse::<u64>() {
                crate::topics::recent_hours(dir, hours, true)
            } else {
                let d = arg_str(args, "days");
                let days = d.parse().unwrap_or(7u64);
                crate::topics::recent(dir, days, true)
            }
        }
        "delete_entry" => {
            let topic = arg_str(args, "topic");
            let idx_str = arg_str(args, "index");
            let m = arg_str(args, "match_str");

            let result = if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::delete::run_by_index(dir, &topic, idx)
            } else if !m.is_empty() {
                crate::delete::run(dir, &topic, false, false, Some(m.as_str()))
            } else {
                crate::delete::run(dir, &topic, true, false, None)
            }?;
            after_write(dir, &topic);
            Ok(result)
        }
        "delete_topic" => {
            let topic = arg_str(args, "topic");
            let result = crate::delete::run(dir, &topic, false, true, None)?;
            crate::cache::invalidate_all();
            let _ = crate::inverted::rebuild(dir);
            load_index(dir);
            Ok(result)
        }
        "append_entry" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            let idx_str = arg_str(args, "index");
            let needle = arg_str(args, "match_str");
            let tag = arg_str(args, "tag");

            let result = if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::edit::append_by_index(dir, &topic, idx, &text)
            } else if !tag.is_empty() {
                crate::edit::append_by_tag(dir, &topic, &tag, &text)
            } else {
                crate::edit::append(dir, &topic, &needle, &text)
            }?;
            after_write(dir, &topic);
            Ok(result)
        }
        "update_entry" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            let idx_str = arg_str(args, "index");
            let needle = arg_str(args, "match_str");

            let result = if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::edit::run_by_index(dir, &topic, idx, &text)
            } else {
                crate::edit::run(dir, &topic, &needle, &text)
            }?;
            after_write(dir, &topic);
            Ok(result)
        }
        "read_topic" => {
            let topic = arg_str(args, "topic");
            let f = crate::config::sanitize_topic(&topic);
            std::fs::read_to_string(dir.join(format!("{f}.md")))
                .map_err(|e| format!("{f}.md: {e}"))
        }
        "digest" => crate::digest::run(dir),
        "list_tags" => crate::stats::list_tags(dir),
        "stats" => crate::stats::stats(dir),
        "list_entries" => {
            let topic = arg_str(args, "topic");
            let m = arg_str(args, "match_str");
            let match_str = if m.is_empty() { None } else { Some(m.as_str()) };
            crate::stats::list_entries(dir, &topic, match_str)
        }
        "prune" => {
            let d = arg_str(args, "days");
            let days = d.parse().unwrap_or(30u64);
            crate::prune::run(dir, days, true)
        }
        "compact" => {
            let topic = arg_str(args, "topic");
            let apply = arg_str(args, "apply") == "true";
            let result = if topic.is_empty() {
                crate::compact::scan(dir)
            } else {
                crate::compact::run(dir, &topic, apply)
            }?;
            if apply {
                crate::cache::invalidate_all();
                let _ = crate::inverted::rebuild(dir);
                load_index(dir);
            }
            Ok(result)
        }
        "export" => crate::export::export(dir),
        "import" => {
            let json = arg_str(args, "json");
            let result = crate::export::import(dir, &json)?;
            crate::cache::invalidate_all();
            let _ = crate::inverted::rebuild(dir);
            load_index(dir);
            Ok(result)
        }
        "xref" => {
            let topic = arg_str(args, "topic");
            crate::xref::refs_for(dir, &topic)
        }
        "migrate" => {
            let apply = arg_str(args, "apply") == "true";
            crate::migrate::run(dir, apply)
        }
        "get_entry" => {
            let topic = arg_str(args, "topic");
            let idx_str = arg_str(args, "index");
            let idx: usize = idx_str.parse()
                .map_err(|_| format!("invalid index: '{idx_str}'"))?;
            crate::stats::get_entry(dir, &topic, idx)
        }
        "rename_topic" => {
            let topic = arg_str(args, "topic");
            let new_name = arg_str(args, "new_name");
            let result = crate::edit::rename_topic(dir, &topic, &new_name)?;
            crate::cache::invalidate_all();
            let _ = crate::inverted::rebuild(dir);
            load_index(dir);
            Ok(result)
        }
        "tag_entry" => {
            let topic = arg_str(args, "topic");
            let idx_str = arg_str(args, "index");
            let needle = arg_str(args, "match_str");
            let add_tags = arg_str(args, "tags");
            let rm_tags = arg_str(args, "remove");
            let idx = if !idx_str.is_empty() {
                Some(idx_str.parse::<usize>().map_err(|_| format!("invalid index: '{idx_str}'"))?)
            } else { None };
            let needle = if needle.is_empty() { None } else { Some(needle.as_str()) };
            let add = if add_tags.is_empty() { None } else { Some(add_tags.as_str()) };
            let rm = if rm_tags.is_empty() { None } else { Some(rm_tags.as_str()) };
            let result = crate::edit::tag_entry(dir, &topic, idx, needle, add, rm)?;
            after_write(dir, &topic);
            Ok(result)
        }
        "rebuild_index" => {
            crate::cache::invalidate_all();
            let result = crate::inverted::rebuild(dir)?;
            load_index(dir);
            Ok(result)
        }
        "index_stats" => {
            let guard = INDEX.lock().map_err(|e| e.to_string())?;
            let data = match guard.as_ref() {
                Some(idx) => std::borrow::Cow::Borrowed(idx.data.as_slice()),
                None => {
                    drop(guard);
                    std::borrow::Cow::Owned(std::fs::read(dir.join("index.bin"))
                        .map_err(|e| format!("index.bin: {e}"))?)
                }
            };
            let mut out = crate::binquery::index_info(&data)?;
            let cache = crate::cache::stats();
            out.push_str(&format!("\n{cache}"));
            Ok(out)
        }
        "index_search" => {
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().unwrap_or(10);
            let guard = INDEX.lock().map_err(|e| e.to_string())?;
            let data = match guard.as_ref() {
                Some(idx) => std::borrow::Cow::Borrowed(idx.data.as_slice()),
                None => {
                    drop(guard);
                    std::borrow::Cow::Owned(std::fs::read(dir.join("index.bin"))
                        .map_err(|e| format!("index.bin: {e}"))?)
                }
            };
            crate::binquery::search(&data, &query, limit)
        }
        "session" => {
            let log = SESSION_LOG.lock().map_err(|e| e.to_string())?;
            if log.is_empty() {
                Ok("no stores this session".into())
            } else {
                let mut out = format!("{} stores this session:\n", log.len());
                for entry in log.iter() {
                    out.push_str(&format!("  {entry}\n"));
                }
                Ok(out)
            }
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn arg_str(args: Option<&Value>, key: &str) -> String {
    args.and_then(|a| a.get(key))
        .map(|v| match v {
            Value::Str(s) => s.clone(),
            Value::Num(n) => n.to_string(),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

fn arg_bool(args: Option<&Value>, key: &str) -> bool {
    let s = arg_str(args, key);
    s == "true" || s == "1"
}
