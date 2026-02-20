use crate::json::Value;
use std::path::Path;

pub fn dispatch(name: &str, args: Option<&Value>, dir: &Path) -> Result<String, String> {
    // Deferred index rebuild: only for read operations.
    // Write ops (store, append, batch_store, delete, etc.) will dirty the index anyway.
    match name {
        "store" | "append" | "batch_store" | "delete" | "append_entry"
        | "update_entry" | "rename_topic" | "merge_topics" | "tag_entry"
        | "import" | "compact_log" | "rebuild_index" | "session" => {}
        _ => super::ensure_index_fresh(dir),
    }
    match name {
        "store" => {
            let topic = arg_ref(args, "topic");
            let text = arg_ref(args, "text");
            let tags = arg_ref(args, "tags");
            let tags = if tags.is_empty() { None } else { Some(tags) };
            let force = arg_bool(args, "force");
            let terse = arg_bool(args, "terse");
            let source = arg_ref(args, "source");
            let source = if source.is_empty() { None } else { Some(source) };
            let conf_str = arg_ref(args, "confidence");
            let confidence = conf_str.parse::<f64>().ok().filter(|c| *c >= 0.0 && *c <= 1.0);
            let links = arg_ref(args, "links");
            let links = if links.is_empty() { None } else { Some(links) };
            let result = crate::store::run_full_ext(dir, topic, text, tags, force, source, confidence, links)?;
            super::after_write(dir, topic);
            super::log_session(format!("[{}] {}", topic,
                result.lines().next().unwrap_or("stored")));
            if terse {
                Ok(result.lines().next().unwrap_or(&result).to_string())
            } else {
                Ok(result)
            }
        }
        "append" => {
            let topic = arg_ref(args, "topic");
            let text = arg_ref(args, "text");
            let result = crate::store::append(dir, topic, text)?;
            super::after_write(dir, topic);
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
            let _lock = crate::lock::FileLock::acquire(dir)?;
            // F3: Open file once, write N entries, fsync once (was N opens + N fsyncs)
            crate::config::ensure_dir(dir)?;
            let log_path = crate::datalog::ensure_log(dir)?;
            let mut log_file = std::fs::OpenOptions::new().append(true).open(&log_path)
                .map_err(|e| format!("open data.log: {e}"))?;
            let mut ok_count = 0;
            let mut results = Vec::new();
            let mut seen: Vec<(String, String)> = Vec::new();
            let mut batch_tokens: Vec<(String, crate::fxhash::FxHashSet<String>)> = Vec::new();
            'batch: for (i, item) in items.iter().enumerate() {
                let topic = item.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let tags = item.get("tags").and_then(|v| v.as_str());
                let source = item.get("source").and_then(|v| v.as_str());
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
                // Token-based semantic dupe check within batch
                let new_tokens: crate::fxhash::FxHashSet<String> = crate::text::tokenize(text)
                    .into_iter().filter(|t| t.len() >= 3).collect();
                if new_tokens.len() >= 6 {
                    let mut is_dupe = false;
                    for (prev_topic, prev_tokens) in &batch_tokens {
                        if *prev_topic != topic { continue; }
                        let intersection = new_tokens.iter()
                            .filter(|t| prev_tokens.contains(*t)).count();
                        let union = new_tokens.len() + prev_tokens.len() - intersection;
                        if union > 0 && intersection as f64 / union as f64 > 0.70 {
                            results.push(format!("  [{}] skipped: similar to earlier batch entry", i + 1));
                            is_dupe = true;
                            break;
                        }
                    }
                    if is_dupe { continue 'batch; }
                    batch_tokens.push((topic.to_string(), new_tokens));
                }
                match crate::store::run_batch_entry_to(&mut log_file, topic, text, tags, source) {
                    Ok(msg) => {
                        ok_count += 1;
                        let first = msg.lines().next().unwrap_or(&msg);
                        results.push(format!("  [{}] {}", i + 1, first));
                        super::log_session(format!("[{}] {}", topic, first));
                    }
                    Err(e) => {
                        let first = e.lines().next().unwrap_or(&e);
                        results.push(format!("  [{}] err: {}", i + 1, first));
                    }
                }
            }
            // Single fsync after all entries written
            if ok_count > 0 {
                let _ = log_file.sync_all();
            }
            drop(log_file);
            drop(_lock);
            if ok_count > 0 {
                super::after_write(dir, "");
            }
            if verbose {
                Ok(format!("batch: {ok_count}/{} stored\n{}", items.len(), results.join("\n")))
            } else {
                Ok(format!("batch: {ok_count}/{} stored", items.len()))
            }
        }
        "search" => {
            let query = arg_ref(args, "query");
            let detail = arg_ref(args, "detail");
            let filter = build_filter(args);
            match detail {
                "count" => crate::search::count(dir, query, &filter),
                "topics" => crate::search::run_topics(dir, query, &filter),
                _ => {
                    let limit = arg_ref(args, "limit").parse::<usize>().ok();
                    let guard = super::INDEX.read().map_err(|e| e.to_string())?;
                    let idx = guard.as_ref().map(|i| i.data.as_slice());
                    let result = match detail {
                        "full" => crate::search::run(dir, query, true, limit, &filter, idx),
                        "brief" => crate::search::run_brief(dir, query, limit, &filter, idx),
                        _ => crate::search::run_medium(dir, query, limit, &filter, idx),
                    };
                    drop(guard);
                    result
                }
            }
        }
        "context" => {
            let q = arg_ref(args, "query");
            let q = if q.is_empty() { None } else { Some(q) };
            let brief = arg_ref(args, "brief");
            if brief == "true" {
                crate::context::run_brief(dir, q, true)
            } else {
                crate::context::run(dir, q, true)
            }
        }
        "topics" => crate::topics::list_compact(dir),
        "recent" => {
            let h = arg_ref(args, "hours");
            if let Ok(hours) = h.parse::<u64>() {
                crate::topics::recent_hours(dir, hours, true)
            } else {
                let d = arg_ref(args, "days");
                let days = d.parse().unwrap_or(7u64);
                crate::topics::recent(dir, days, true)
            }
        }
        "delete" => {
            let topic = arg_ref(args, "topic");
            let all = arg_bool(args, "all");
            let result = if all {
                crate::delete::run(dir, topic, false, true, None)
            } else {
                let idx_str = arg_ref(args, "index");
                let m = arg_ref(args, "match_str");
                if !idx_str.is_empty() {
                    let idx: usize = idx_str.parse()
                        .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                    crate::delete::run_by_index(dir, topic, idx)
                } else if !m.is_empty() {
                    crate::delete::run(dir, topic, false, false, Some(m))
                } else {
                    crate::delete::run(dir, topic, true, false, None)
                }
            }?;
            super::after_write(dir, topic);
            Ok(result)
        }
        "append_entry" => {
            let topic = arg_ref(args, "topic");
            let text = arg_ref(args, "text");
            let idx_str = arg_ref(args, "index");
            let needle = arg_ref(args, "match_str");
            let tag = arg_ref(args, "tag");
            let result = if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::edit::append_by_index(dir, topic, idx, text)
            } else if !tag.is_empty() {
                crate::edit::append_by_tag(dir, topic, tag, text)
            } else {
                crate::edit::append(dir, topic, needle, text)
            }?;
            super::after_write(dir, topic);
            Ok(result)
        }
        "update_entry" => {
            let topic = arg_ref(args, "topic");
            let text = arg_ref(args, "text");
            let idx_str = arg_ref(args, "index");
            let needle = arg_ref(args, "match_str");
            let result = if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::edit::run_by_index(dir, topic, idx, text)
            } else {
                crate::edit::run(dir, topic, needle, text)
            }?;
            super::after_write(dir, topic);
            Ok(result)
        }
        "read_topic" => {
            let topic = arg_ref(args, "topic");
            crate::topics::read_topic(dir, topic)
        }
        "digest" => crate::digest::run(dir),
        "list_tags" => crate::stats::list_tags(dir),
        "stats" => crate::stats::stats_fast(dir),
        "list_entries" => {
            let topic = arg_ref(args, "topic");
            let m = arg_ref(args, "match_str");
            let match_str = if m.is_empty() { None } else { Some(m) };
            crate::stats::list_entries(dir, topic, match_str)
        }
        "prune" => {
            let d = arg_ref(args, "days");
            let days = d.parse().unwrap_or(30u64);
            crate::prune::run(dir, days, true)
        }
        "compact" => {
            let log = arg_bool(args, "log");
            if log {
                let result = crate::datalog::compact_log(dir)?;
                super::after_write(dir, "");
                return Ok(result);
            }
            let topic = arg_ref(args, "topic");
            let apply = arg_ref(args, "apply") == "true";
            let result = if topic.is_empty() {
                crate::compact::scan(dir)
            } else {
                crate::compact::run(dir, topic, apply)
            }?;
            if apply { super::after_write(dir, ""); }
            Ok(result)
        }
        "export" => crate::export::export(dir),
        "import" => {
            let json = arg_ref(args, "json");
            let result = crate::export::import(dir, json)?;
            super::after_write(dir, "");
            Ok(result)
        }
        "xref" => {
            let topic = arg_ref(args, "topic");
            crate::xref::refs_for(dir, topic)
        }
        "migrate" => {
            let apply = arg_ref(args, "apply") == "true";
            crate::migrate::run(dir, apply)
        }
        "get_entry" => {
            let topic = arg_ref(args, "topic");
            let idx_str = arg_ref(args, "index");
            let idx: usize = idx_str.parse()
                .map_err(|_| format!("invalid index: '{idx_str}'"))?;
            crate::stats::get_entry(dir, topic, idx)
        }
        "rename_topic" => {
            let topic = arg_ref(args, "topic");
            let new_name = arg_ref(args, "new_name");
            let result = crate::edit::rename_topic(dir, topic, new_name)?;
            super::after_write(dir, new_name);
            Ok(result)
        }
        "merge_topics" => {
            let from = arg_ref(args, "from");
            let into = arg_ref(args, "into");
            let result = crate::edit::merge_topics(dir, from, into)?;
            super::after_write(dir, into);
            Ok(result)
        }
        "tag_entry" => {
            let topic = arg_ref(args, "topic");
            let idx_str = arg_ref(args, "index");
            let needle = arg_ref(args, "match_str");
            let add_tags = arg_ref(args, "tags");
            let rm_tags = arg_ref(args, "remove");
            let idx = if !idx_str.is_empty() {
                Some(idx_str.parse::<usize>().map_err(|_| format!("invalid index: '{idx_str}'"))?)
            } else { None };
            let needle = if needle.is_empty() { None } else { Some(needle) };
            let add = if add_tags.is_empty() { None } else { Some(add_tags) };
            let rm = if rm_tags.is_empty() { None } else { Some(rm_tags) };
            let result = crate::edit::tag_entry(dir, topic, idx, needle, add, rm)?;
            super::after_write(dir, topic);
            Ok(result)
        }
        "rebuild_index" => {
            let (result, bytes) = crate::inverted::rebuild_and_persist(dir)?;
            super::store_index(bytes);
            Ok(result)
        }
        "index_stats" => {
            let guard = super::INDEX.read().map_err(|e| e.to_string())?;
            let data = match guard.as_ref() {
                Some(idx) => std::borrow::Cow::Borrowed(idx.data.as_slice()),
                None => {
                    drop(guard);
                    std::borrow::Cow::Owned(std::fs::read(dir.join("index.bin"))
                        .map_err(|e| format!("index.bin: {e}"))?)
                }
            };
            crate::binquery::index_info(&data)
        }
        "index_search" => {
            let query = arg_ref(args, "query");
            let limit = arg_ref(args, "limit").parse::<usize>().unwrap_or(10);
            let guard = super::INDEX.read().map_err(|e| e.to_string())?;
            let data = match guard.as_ref() {
                Some(idx) => std::borrow::Cow::Borrowed(idx.data.as_slice()),
                None => {
                    drop(guard);
                    std::borrow::Cow::Owned(std::fs::read(dir.join("index.bin"))
                        .map_err(|e| format!("index.bin: {e}"))?)
                }
            };
            crate::binquery::search(&data, query, limit)
        }
        "session" => {
            let log = super::SESSION_LOG.lock().map_err(|e| e.to_string())?;
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
        "search_entity" => {
            let query = arg_ref(args, "query");
            let limit = arg_ref(args, "limit").parse::<usize>().ok();
            let filter = build_filter(args);
            let guard = super::INDEX.read().map_err(|e| e.to_string())?;
            let idx = guard.as_ref().map(|i| i.data.as_slice());
            let result = crate::search::run_grouped(dir, query, limit, &filter, idx);
            drop(guard);
            result
        }
        "reconstruct" => {
            let query = arg_ref(args, "query");
            let detail = arg_ref(args, "detail");
            let detail = if detail.is_empty() { "summary" } else { detail };
            let since_str = arg_str(args, "since");
            let since_hours = since_str.parse::<u64>().ok();
            crate::reconstruct::run(dir, query, detail, since_hours)
        }
        "compact_log" => {
            let result = crate::datalog::compact_log(dir)?;
            super::after_write(dir, "");
            Ok(result)
        }
        "callgraph" => {
            let pattern = arg_str(args, "pattern");
            let path_str = arg_str(args, "path");
            let glob = arg_str(args, "glob");
            let glob = if glob.is_empty() { "*.rs" } else { glob.as_str() };
            let depth = arg_str(args, "depth").parse::<usize>().unwrap_or(2);
            let direction = arg_str(args, "direction");
            let direction = if direction.is_empty() { "both" } else { direction.as_str() };
            let result = crate::callgraph::run(&pattern, std::path::Path::new(&path_str), glob, depth, direction)?;
            let store_topic = arg_str(args, "store_topic");
            if !store_topic.is_empty() {
                let tags = arg_str(args, "tags");
                let tags = if tags.is_empty() { "structural,callgraph,raw-data" } else { tags.as_str() };
                let source = format!("{}/**/{}", path_str, glob);
                crate::store::run_full(dir, &store_topic, &result, Some(tags), true, Some(&source))?;
                super::after_write(dir, &store_topic);
            }
            Ok(result)
        }
        "codepath" => {
            let pattern = arg_str(args, "pattern");
            let path_str = arg_str(args, "path");
            let glob = arg_str(args, "glob");
            let glob = if glob.is_empty() { "*.rs" } else { glob.as_str() };
            let ctx = arg_str(args, "context").parse::<usize>().unwrap_or(2);
            let result = crate::codepath::run(&pattern, std::path::Path::new(&path_str), glob, ctx)?;
            let store_topic = arg_str(args, "store_topic");
            if !store_topic.is_empty() {
                let tags = arg_str(args, "tags");
                let tags = if tags.is_empty() { "structural,coupling,raw-data" } else { tags.as_str() };
                let source = format!("{}/**/{}", path_str, glob);
                crate::store::run_full(dir, &store_topic, &result, Some(tags), true, Some(&source))?;
                super::after_write(dir, &store_topic);
            }
            Ok(result)
        }
        "dep_graph" => crate::depgraph::run(dir),
        "check_stale" => {
            let refresh = arg_bool(args, "refresh");
            if refresh {
                crate::stats::refresh_stale(dir)
            } else {
                crate::stats::check_stale(dir)
            }
        }
        "refresh_stale" => crate::stats::refresh_stale(dir),
        _ => Err(format!("unknown tool: {name}")),
    }
}

/// Borrow string value from args — zero allocation for the common case (string values).
/// Returns "" if key missing or value is not a string.
fn arg_ref<'a>(args: Option<&'a Value>, key: &str) -> &'a str {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// Owned string from args — needed for Num/Bool value conversion.
fn arg_str(args: Option<&Value>, key: &str) -> String {
    args.and_then(|a| a.get(key))
        .map(|v| match v {
            Value::Str(s) => s.clone(),
            Value::Num(n) => if n.fract() == 0.0 { format!("{}", *n as i64) } else { n.to_string() },
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

fn arg_bool(args: Option<&Value>, key: &str) -> bool {
    let s = arg_ref(args, key);
    s == "true" || s == "1"
}

fn build_filter(args: Option<&Value>) -> crate::search::Filter {
    let after_raw = arg_ref(args, "after");
    let before_raw = arg_ref(args, "before");
    let after = crate::time::resolve_date_shortcut(after_raw);
    let before = crate::time::resolve_date_shortcut(before_raw);
    let tag = arg_ref(args, "tag");
    let topic = arg_ref(args, "topic");
    let mode = match arg_ref(args, "mode") {
        "or" => crate::search::SearchMode::Or,
        _ => crate::search::SearchMode::And,
    };
    crate::search::Filter {
        after: if after.is_empty() { None } else { crate::time::parse_date_days(&after) },
        before: if before.is_empty() { None } else { crate::time::parse_date_days(&before) },
        tag: if tag.is_empty() { None } else { Some(tag.to_string()) },
        topic: if topic.is_empty() { None } else { Some(topic.to_string()) },
        mode,
    }
}
