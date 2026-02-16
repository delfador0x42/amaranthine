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
    std::env::set_var("AMARANTHINE_REEXEC", "1");
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
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
            ("version".into(), Value::Str("0.6.0".into())),
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

fn tool_list() -> Value {
    Value::Arr(vec![
        tool("store", "Store a timestamped knowledge entry under a topic. Warns on duplicate content.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Entry content")]),
        tool("append", "Add text to the last entry in a topic (no new timestamp). Use when adding related info to a recent entry.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Text to append")]),
        tool("search", "Search all knowledge files (case-insensitive, returns full sections)",
            &["query"],
            &[("query", "string", "Search query"),
              ("limit", "string", "Max results to return (default: unlimited)")]),
        tool("search_brief", "Quick search: just topic names + first matching line per hit",
            &["query"],
            &[("query", "string", "Search query"),
              ("limit", "string", "Max results to return (default: unlimited)")]),
        tool("search_count", "Count matching sections without returning content. Fast way to gauge query scope.",
            &["query"],
            &[("query", "string", "Search query")]),
        tool("context", "Session briefing: topics + recent entries (7 days) + optional search",
            &[],
            &[("query", "string", "Optional search query"),
              ("brief", "string", "Set to 'true' for compact mode (topics only, no recent)")]),
        tool("topics", "List all topic files with entry and line counts",
            &[], &[]),
        tool("recent", "Show entries from last N days across all topics",
            &[],
            &[("days", "string", "Number of days (default: 7)")]),
        tool("delete_entry", "Remove the most recent entry from a topic",
            &["topic"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Delete entry matching this substring instead of last")]),
        tool("delete_topic", "Delete an entire topic and all its entries",
            &["topic"],
            &[("topic", "string", "Topic name")]),
        tool("append_entry", "Add text to an existing entry found by substring match (keeps timestamp, preserves body)",
            &["topic", "match_str", "text"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Substring to find the entry to append to"),
              ("text", "string", "Text to append to the entry")]),
        tool("update_entry", "Overwrite an existing entry's text (keeps timestamp)",
            &["topic", "match_str", "text"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Substring to find the entry to update"),
              ("text", "string", "Replacement text for the entry")]),
        tool("read_topic", "Read the full contents of a specific topic file",
            &["topic"],
            &[("topic", "string", "Topic name")]),
        tool("digest", "Compact summary of all topics (one bullet per entry)",
            &[], &[]),
        tool("_reload", "Re-exec the server binary to pick up code changes. Sends tools/list_changed notification after reload.",
            &[], &[]),
    ])
}

fn dispatch(name: &str, args: Option<&Value>, dir: &Path) -> Result<String, String> {
    match name {
        "store" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            crate::store::run(dir, &topic, &text)
        }
        "append" => {
            let topic = arg_str(args, "topic");
            let text = arg_str(args, "text");
            crate::store::append(dir, &topic, &text)
        }
        "search" => {
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().ok();
            crate::search::run(dir, &query, true, limit)
        }
        "search_brief" => {
            let query = arg_str(args, "query");
            let limit = arg_str(args, "limit").parse::<usize>().ok();
            crate::search::run_brief(dir, &query, limit)
        }
        "search_count" => {
            let query = arg_str(args, "query");
            crate::search::count(dir, &query)
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
            let d = arg_str(args, "days");
            let days = d.parse().unwrap_or(7u64);
            crate::topics::recent(dir, days, true)
        }
        "delete_entry" => {
            let topic = arg_str(args, "topic");
            let m = arg_str(args, "match_str");
            let match_str = if m.is_empty() { None } else { Some(m.as_str()) };
            crate::delete::run(dir, &topic, match_str.is_none(), false, match_str)
        }
        "delete_topic" => {
            let topic = arg_str(args, "topic");
            crate::delete::run(dir, &topic, false, true, None)
        }
        "append_entry" => {
            let topic = arg_str(args, "topic");
            let needle = arg_str(args, "match_str");
            let text = arg_str(args, "text");
            crate::edit::append(dir, &topic, &needle, &text)
        }
        "update_entry" => {
            let topic = arg_str(args, "topic");
            let needle = arg_str(args, "match_str");
            let text = arg_str(args, "text");
            crate::edit::run(dir, &topic, &needle, &text)
        }
        "read_topic" => {
            let topic = arg_str(args, "topic");
            let f = crate::config::sanitize_topic(&topic);
            std::fs::read_to_string(dir.join(format!("{f}.md")))
                .map_err(|e| format!("{f}.md: {e}"))
        }
        "digest" => crate::digest::run(dir),
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn arg_str(args: Option<&Value>, key: &str) -> String {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}
