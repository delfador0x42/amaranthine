use crate::json::Value;
use std::io::{self, BufRead, Write as _};
use std::path::Path;

pub fn run(dir: &Path) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();

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
                // exec only returns on failure — keep running
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
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
    Ok(())
}

fn do_reload() {
    use std::os::unix::process::CommandExt;

    // If running from an installed location, copy fresh build + codesign
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let src = exe.parent()
        .and_then(|p| p.parent())
        .and_then(|_| {
            // Find the cargo project root relative to the binary
            let manifest = std::env::var("AMARANTHINE_SRC").ok()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    // Default: look for target/release relative to common locations
                    let home = std::env::var("HOME").unwrap_or_default();
                    std::path::PathBuf::from(home).join("wudan/dojo/crash3/amaranthine")
                });
            let release = manifest.join("target/release/amaranthine");
            if release.exists() { Some(release) } else { None }
        });

    if let Some(src_bin) = src {
        // Copy new binary over the running one
        if let Err(e) = std::fs::copy(&src_bin, &exe) {
            eprintln!("reload: copy failed: {e}");
        } else {
            // Ad-hoc codesign so taskgate doesn't kill it
            let _ = std::process::Command::new("codesign")
                .args(["-s", "-", "-f"])
                .arg(&exe)
                .output();
        }
    }

    std::env::set_var("AMARANTHINE_REEXEC", "1");
    let args: Vec<String> = std::env::args().skip(1).collect();
    // exec replaces this process — only returns on failure
    let _err = std::process::Command::new(&exe).args(&args).exec();
    // If we get here, exec failed — remove env var and continue
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
            ("version".into(), Value::Str("1.4.0".into())),
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

/// Shared search filter properties for tool definitions.
const SEARCH_FILTER_PROPS: &[(&str, &str, &str)] = &[
    ("limit", "string", "Max results to return (default: unlimited)"),
    ("after", "string", "Only entries on/after date (YYYY-MM-DD or 'today'/'yesterday'/'this-week')"),
    ("before", "string", "Only entries on/before date (YYYY-MM-DD or 'today'/'yesterday')"),
    ("tag", "string", "Only entries with this tag"),
    ("mode", "string", "Search mode: 'and' (default, all terms must match) or 'or' (any term matches)"),
];

fn tool_list() -> Value {
    // Build search props: query + shared filter props
    let search_props: Vec<(&str, &str, &str)> = std::iter::once(("query", "string", "Search query"))
        .chain(SEARCH_FILTER_PROPS.iter().copied())
        .collect();

    // Count-only doesn't need limit
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
        tool("batch_store", "Store multiple entries in one call. Each entry: {topic, text, tags?}. Faster than sequential store calls.",
            &["entries"],
            &[("entries", "string", "JSON array of objects: [{\"topic\":\"...\",\"text\":\"...\",\"tags\":\"...\"}]"),
              ("terse", "string", "Set to 'true' for minimal response (just count)")]),
        tool("search", "Search all knowledge files (case-insensitive). Splits CamelCase/snake_case. Falls back to OR when AND finds nothing.",
            &["query"], &search_props),
        tool("search_brief", "Quick search: just topic names + first matching line per hit",
            &["query"], &search_props),
        tool("search_medium", "Medium search: topic + timestamp + first 2 content lines per hit. Between brief and full.",
            &["query"], &search_props),
        tool("search_count", "Count matching sections without returning content. Fast way to gauge query scope.",
            &["query"], &search_count_props),
        tool("search_topics", "Show which topics matched and how many hits per topic. Best first step before deep search.",
            &["query"], &search_count_props),
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
        tool("append_entry", "Add text to an existing entry found by substring match or index (keeps timestamp, preserves body)",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Substring to find the entry to append to"),
              ("index", "string", "Entry index number (from list_entries)"),
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
        tool("_reload", "Re-exec the server binary to pick up code changes. Sends tools/list_changed notification after reload.",
            &[], &[]),
    ])
}

fn build_filter(args: Option<&Value>) -> crate::search::Filter {
    let after = resolve_date_shortcut(&arg_str(args, "after"));
    let before = resolve_date_shortcut(&arg_str(args, "before"));
    let tag = arg_str(args, "tag");
    let mode = match arg_str(args, "mode").as_str() {
        "or" => crate::search::SearchMode::Or,
        _ => crate::search::SearchMode::And,
    };
    crate::search::Filter {
        after: if after.is_empty() { None } else { crate::time::parse_date_days(&after) },
        before: if before.is_empty() { None } else { crate::time::parse_date_days(&before) },
        tag: if tag.is_empty() { None } else { Some(tag) },
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
    // Inverse of civil_to_days (Howard Hinnant's algorithm)
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

fn dispatch(name: &str, args: Option<&Value>, dir: &Path) -> Result<String, String> {
    match name {
        "store" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            let tags = arg_str(args, "tags");
            let tags = if tags.is_empty() { None } else { Some(tags.as_str()) };
            let force = arg_bool(args, "force");
            let terse = arg_bool(args, "terse");
            let result = crate::store::run_full(dir, &topic, &text, tags, force)?;
            if terse {
                Ok(result.lines().next().unwrap_or(&result).to_string())
            } else {
                Ok(result)
            }
        }
        "append" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            crate::store::append(dir, &topic, &text)
        }
        "batch_store" => {
            let raw = arg_str(args, "entries");
            let terse = arg_bool(args, "terse");
            let arr = crate::json::parse(&raw).map_err(|e| format!("invalid JSON: {e}"))?;
            let items = match &arr {
                Value::Arr(v) => v,
                _ => return Err("entries must be a JSON array".into()),
            };
            let mut ok_count = 0;
            let mut results = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let topic = item.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let tags = item.get("tags").and_then(|v| v.as_str());
                if topic.is_empty() || text.is_empty() {
                    results.push(format!("  [{}] skipped: missing topic or text", i + 1));
                    continue;
                }
                match crate::store::run_full(dir, topic, text, tags, false) {
                    Ok(msg) => {
                        ok_count += 1;
                        let first = msg.lines().next().unwrap_or(&msg);
                        results.push(format!("  [{}] {}", i + 1, first));
                    }
                    Err(e) => {
                        let first = e.lines().next().unwrap_or(&e);
                        results.push(format!("  [{}] {}", i + 1, first));
                    }
                }
            }
            if terse {
                Ok(format!("batch: {ok_count}/{} stored", items.len()))
            } else {
                Ok(format!("batch: {ok_count}/{} stored\n{}", items.len(), results.join("\n")))
            }
        }
        "search" => {
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().ok();
            let filter = build_filter(args);
            crate::search::run(dir, &query, true, limit, &filter)
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

            if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::delete::run_by_index(dir, &topic, idx)
            } else if !m.is_empty() {
                crate::delete::run(dir, &topic, false, false, Some(m.as_str()))
            } else {
                crate::delete::run(dir, &topic, true, false, None)
            }
        }
        "delete_topic" => {
            let topic = arg_str(args, "topic");
            crate::delete::run(dir, &topic, false, true, None)
        }
        "append_entry" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            let idx_str = arg_str(args, "index");
            let needle = arg_str(args, "match_str");

            if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::edit::append_by_index(dir, &topic, idx, &text)
            } else {
                crate::edit::append(dir, &topic, &needle, &text)
            }
        }
        "update_entry" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            let idx_str = arg_str(args, "index");
            let needle = arg_str(args, "match_str");

            if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::edit::run_by_index(dir, &topic, idx, &text)
            } else {
                crate::edit::run(dir, &topic, &needle, &text)
            }
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
            if topic.is_empty() {
                crate::compact::scan(dir)
            } else {
                crate::compact::run(dir, &topic, apply)
            }
        }
        "export" => crate::export::export(dir),
        "import" => {
            let json = arg_str(args, "json");
            crate::export::import(dir, &json)
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
            crate::edit::rename_topic(dir, &topic, &new_name)
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
            crate::edit::tag_entry(dir, &topic, idx, needle, add, rm)
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
