use crate::json::Value;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn run(dir: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.is_empty() { continue; }
        let msg = match crate::json::parse(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id");

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
                Some(match dispatch(name, args, dir, &exe) {
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

fn init_result() -> Value {
    Value::Obj(vec![
        ("protocolVersion".into(), Value::Str("2024-11-05".into())),
        ("capabilities".into(), Value::Obj(vec![
            ("tools".into(), Value::Obj(Vec::new())),
        ])),
        ("serverInfo".into(), Value::Obj(vec![
            ("name".into(), Value::Str("amaranthine".into())),
            ("version".into(), Value::Str("0.4.0".into())),
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
        tool("store", "Store a timestamped knowledge entry under a topic",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Entry content")]),
        tool("search", "Search all knowledge files (case-insensitive, returns full sections)",
            &["query"],
            &[("query", "string", "Search query")]),
        tool("context", "Session briefing: topics + recent entries (7 days) + optional search",
            &[],
            &[("query", "string", "Optional search query")]),
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
    ])
}

fn dispatch(name: &str, args: Option<&Value>, dir: &Path, exe: &Path) -> Result<String, String> {
    // read_topic: direct file read, no subprocess needed
    if name == "read_topic" {
        let topic = arg_str(args, "topic");
        let f = crate::config::sanitize_topic(&topic);
        return std::fs::read_to_string(dir.join(format!("{f}.md")))
            .map_err(|e| format!("{f}.md: {e}"));
    }

    let d = dir.to_string_lossy().into_owned();
    let mut cli: Vec<String> = vec!["--plain".into(), "-d".into(), d];

    match name {
        "store" => cli.extend(["store".into(), arg_str(args, "topic"), arg_str(args, "text")]),
        "search" => cli.extend(["search".into(), arg_str(args, "query")]),
        "context" => {
            cli.push("context".into());
            let q = arg_str(args, "query");
            if !q.is_empty() { cli.push(q); }
        }
        "topics" => cli.push("topics".into()),
        "recent" => {
            cli.push("recent".into());
            let d = arg_str(args, "days");
            if !d.is_empty() { cli.push(d); }
        }
        "delete_entry" => {
            let m = arg_str(args, "match_str");
            cli.extend(["delete".into(), arg_str(args, "topic")]);
            if m.is_empty() {
                cli.push("--last".into());
            } else {
                cli.extend(["--match".into(), m]);
            }
        }
        "delete_topic" => cli.extend(["delete".into(), arg_str(args, "topic"), "--all".into()]),
        "update_entry" => {
            cli.extend([
                "edit".into(), arg_str(args, "topic"),
                "--match".into(), arg_str(args, "match_str"),
                arg_str(args, "text"),
            ]);
        }
        "digest" => cli.push("digest".into()),
        _ => return Err(format!("unknown tool: {name}")),
    }

    let out = Command::new(exe)
        .args(&cli)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;

    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).into_owned()
        } else {
            err.into_owned()
        })
    }
}

fn arg_str(args: Option<&Value>, key: &str) -> String {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}
