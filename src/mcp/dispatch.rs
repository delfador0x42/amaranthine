use crate::json::Value;
use std::path::Path;

pub fn dispatch(name: &str, args: Option<&Value>, dir: &Path) -> Result<String, String> {
    match name {
        "store" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            let tags = arg_str(args, "tags");
            let tags = if tags.is_empty() { None } else { Some(tags.as_str()) };
            let force = arg_bool(args, "force");
            let terse = arg_bool(args, "terse");
            let source = arg_str(args, "source");
            let source = if source.is_empty() { None } else { Some(source.as_str()) };
            let result = crate::store::run_full(dir, &topic, &text, tags, force, source)?;
            super::after_write(dir, &topic);
            super::log_session(format!("[{}] {}", topic,
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
            super::after_write(dir, &topic);
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
            let mut ok_count = 0;
            let mut results = Vec::new();
            let mut seen: Vec<(String, String)> = Vec::new();
            for (i, item) in items.iter().enumerate() {
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
                match crate::store::run_batch_entry(dir, topic, text, tags, source) {
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
            drop(_lock);
            if ok_count > 0 {
                let _ = crate::inverted::rebuild(dir);
                super::load_index(dir);
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
            super::after_write(dir, &topic);
            Ok(result)
        }
        "delete_topic" => {
            let topic = arg_str(args, "topic");
            let result = crate::delete::run(dir, &topic, false, true, None)?;
            let _ = crate::inverted::rebuild(dir);
            super::load_index(dir);
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
            super::after_write(dir, &topic);
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
            super::after_write(dir, &topic);
            Ok(result)
        }
        "read_topic" => {
            let topic = arg_str(args, "topic");
            crate::topics::read_topic(dir, &topic)
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
                let _ = crate::inverted::rebuild(dir);
                super::load_index(dir);
            }
            Ok(result)
        }
        "export" => crate::export::export(dir),
        "import" => {
            let json = arg_str(args, "json");
            let result = crate::export::import(dir, &json)?;
            let _ = crate::inverted::rebuild(dir);
            super::load_index(dir);
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
            let _ = crate::inverted::rebuild(dir);
            super::load_index(dir);
            Ok(result)
        }
        "merge_topics" => {
            let from = arg_str(args, "from");
            let into = arg_str(args, "into");
            let result = crate::edit::merge_topics(dir, &from, &into)?;
            let _ = crate::inverted::rebuild(dir);
            super::load_index(dir);
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
            super::after_write(dir, &topic);
            Ok(result)
        }
        "rebuild_index" => {
            let result = crate::inverted::rebuild(dir)?;
            super::load_index(dir);
            Ok(result)
        }
        "index_stats" => {
            let guard = super::INDEX.lock().map_err(|e| e.to_string())?;
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
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().unwrap_or(10);
            let guard = super::INDEX.lock().map_err(|e| e.to_string())?;
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
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().ok();
            let filter = build_filter(args);
            crate::search::run_grouped(dir, &query, limit, &filter)
        }
        "reconstruct" => {
            let query = arg_str(args, "query");
            crate::reconstruct::run(dir, &query)
        }
        "dep_graph" => crate::depgraph::run(dir),
        "check_stale" => crate::stats::check_stale(dir),
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

fn build_filter(args: Option<&Value>) -> crate::search::Filter {
    let after = crate::time::resolve_date_shortcut(&arg_str(args, "after"));
    let before = crate::time::resolve_date_shortcut(&arg_str(args, "before"));
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
