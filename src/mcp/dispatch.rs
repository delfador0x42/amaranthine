use crate::json::Value;
use std::path::Path;

pub fn dispatch(name: &str, args: Option<&Value>, dir: &Path) -> Result<String, String> {
    // Deferred index rebuild: only for read operations.
    // Write ops (store, append, batch, delete, etc.) will dirty the index anyway.
    match name {
        "store" | "append" | "batch" | "delete" | "append_entry"
        | "revise" | "rename" | "merge" | "tag"
        | "import" | "reindex" | "session" => {}
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
        "append" | "append_entry" => {
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
            } else if !needle.is_empty() {
                crate::edit::append(dir, topic, needle, text)
            } else {
                crate::store::append(dir, topic, text)
            }?;
            super::after_write(dir, topic);
            Ok(result)
        }
        "batch" => {
            let verbose = arg_bool(args, "verbose");
            let items = args.and_then(|a| a.get("entries"))
                .and_then(|v| match v { Value::Arr(a) => Some(a), _ => None })
                .ok_or("entries must be an array")?;
            if items.len() > 30 {
                return Err(format!(
                    "batch too large ({} entries, max 30). Split into smaller batch calls.",
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
                "grouped" => {
                    let limit = arg_ref(args, "limit").parse::<usize>().ok();
                    let guard = super::INDEX.read().map_err(|e| e.to_string())?;
                    let idx = guard.as_ref().map(|i| i.data.as_slice());
                    let result = crate::search::run_grouped(dir, query, limit, &filter, idx);
                    drop(guard);
                    result
                }
                "index" => {
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
            // Legacy: redirect to brief
            let q = arg_ref(args, "query");
            let q = if q.is_empty() { None } else { Some(q) };
            let brief = arg_bool(args, "brief");
            crate::context::run_inner_pub(dir, q, true, brief)
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
        "revise" => {
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
        "read" => {
            let topic = arg_ref(args, "topic");
            crate::topics::read_topic(dir, topic)
        }
        "stats" => {
            let detail = arg_ref(args, "detail");
            match detail {
                "tags" => crate::stats::list_tags(dir),
                "index" => {
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
                _ => crate::stats::stats_fast(dir),
            }
        }
        "entries" => {
            let topic = arg_ref(args, "topic");
            let idx_str = arg_ref(args, "index");
            if !idx_str.is_empty() {
                let idx: usize = idx_str.parse()
                    .map_err(|_| format!("invalid index: '{idx_str}'"))?;
                crate::stats::get_entry(dir, topic, idx)
            } else {
                let m = arg_ref(args, "match_str");
                let match_str = if m.is_empty() { None } else { Some(m) };
                crate::stats::list_entries(dir, topic, match_str)
            }
        }
        "prune" => {
            let d = arg_ref(args, "days");
            let days = d.parse().unwrap_or(30u64);
            crate::prune::run(dir, days, true)
        }
        "compact" => {
            let mode = arg_ref(args, "mode");
            if mode == "migrate" {
                let apply = arg_ref(args, "apply") == "true";
                return crate::migrate::run(dir, apply);
            }
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
        "rename" => {
            let topic = arg_ref(args, "topic");
            let new_name = arg_ref(args, "new_name");
            let result = crate::edit::rename_topic(dir, topic, new_name)?;
            super::after_write(dir, new_name);
            Ok(result)
        }
        "merge" => {
            let from = arg_ref(args, "from");
            let into = arg_ref(args, "into");
            let result = crate::edit::merge_topics(dir, from, into)?;
            super::after_write(dir, into);
            Ok(result)
        }
        "tag" => {
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
        "reindex" => {
            let (result, bytes) = crate::inverted::rebuild_and_persist(dir)?;
            super::store_index(bytes);
            Ok(result)
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
        "brief" => {
            let query = arg_ref(args, "query");
            if query.is_empty() {
                // No query → meta-briefing (session start overview)
                let compact = arg_bool(args, "compact");
                crate::context::run_inner_pub(dir, None, true, compact)
            } else {
                let detail = arg_ref(args, "detail");
                let detail = if detail.is_empty() { "summary" } else { detail };
                let since_str = arg_str(args, "since");
                let since_hours = since_str.parse::<u64>().ok();
                let focus_str = arg_ref(args, "focus");
                let focus = if focus_str.is_empty() { None } else { Some(focus_str) };
                crate::reconstruct::run(dir, query, detail, since_hours, focus)
            }
        }
        "trace" => {
            let mode = arg_ref(args, "mode");
            let pattern = arg_str(args, "pattern");
            let path_str = arg_str(args, "path");
            let glob = arg_str(args, "glob");
            let glob = if glob.is_empty() { "*.rs" } else { glob.as_str() };
            let p = Path::new(&path_str);
            let result = match mode {
                "codepath" => {
                    let ctx = arg_str(args, "context").parse::<usize>().unwrap_or(2);
                    crate::codepath::run(&pattern, p, glob, ctx)?
                }
                "reverse" => crate::reverse::reverse(p, glob)?,
                "core" => {
                    let entry = arg_str(args, "entry");
                    let entry = if entry.is_empty() { "main|run" } else { entry.as_str() };
                    crate::reverse::core(p, glob, entry)?
                }
                "simplify" => crate::reverse::simplify(p, glob)?,
                "crash" => {
                    let input = arg_str(args, "pattern");
                    crate::crash::run(&input, p, glob)?
                }
                "perf" => {
                    let entry = arg_str(args, "entry");
                    if entry.is_empty() {
                        return Err("entry function name is required for perf mode".into());
                    }
                    let depth = arg_str(args, "depth").parse::<usize>().unwrap_or(3);
                    crate::perf::run(p, glob, &entry, depth)?
                }
                _ => {
                    if pattern.is_empty() {
                        return Err("pattern is required for callgraph mode".into());
                    }
                    let depth = arg_str(args, "depth").parse::<usize>().unwrap_or(2);
                    let direction = arg_str(args, "direction");
                    let direction = if direction.is_empty() { "both" } else { direction.as_str() };
                    crate::callgraph::run(&pattern, p, glob, depth, direction)?
                }
            };
            let store_topic = arg_str(args, "store_topic");
            if !store_topic.is_empty() {
                let tags_str = arg_str(args, "tags");
                let default_tags = match mode {
                    "codepath" => "structural,coupling,raw-data",
                    "reverse" => "architecture,structural",
                    "core" => "architecture,reachability",
                    "simplify" => "architecture,simplification",
                    "crash" => "debugging,crash-analysis",
                    "perf" => "performance,antipattern",
                    _ => "structural,callgraph,raw-data",
                };
                let tags = if tags_str.is_empty() { default_tags } else { tags_str.as_str() };
                let source = format!("{}/**/{}", path_str, glob);
                crate::store::run_full(dir, &store_topic, &result, Some(tags), true, Some(&source))?;
                super::after_write(dir, &store_topic);
            }
            Ok(result)
        }
        "graph" => {
            let focus = arg_ref(args, "focus");
            if focus.is_empty() { crate::depgraph::run(dir) }
            else { crate::depgraph::run_focused(dir, focus) }
        }
        "stale" => {
            let refresh = arg_bool(args, "refresh");
            if refresh {
                crate::stats::refresh_stale(dir)
            } else {
                crate::stats::check_stale(dir)
            }
        }
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
    // days/hours shortcuts: convert to after= date if after is not set
    let after = if after_raw.is_empty() {
        let days = arg_ref(args, "days").parse::<u64>().ok();
        let hours = arg_ref(args, "hours").parse::<u64>().ok();
        crate::time::relative_to_date(days, hours).unwrap_or_default()
    } else {
        crate::time::resolve_date_shortcut(after_raw)
    };
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
