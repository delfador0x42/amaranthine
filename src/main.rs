mod config;
mod context;
mod delete;
mod digest;
mod edit;
mod index;
mod install;
mod json;
mod mcp;
mod prune;
mod search;
mod store;
mod time;
mod topics;

use std::env;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut dir_override: Option<String> = None;
    let mut plain = false;
    let mut cmd_start = 0;
    let mut i = 0;

    while i < args.len() {
        let a = &args[i];
        if a == "-d" || a == "--dir" {
            dir_override = args.get(i + 1).cloned();
            i += 2;
            cmd_start = i;
        } else if a.starts_with("-d=") || a.starts_with("--dir=") {
            dir_override = a.splitn(2, '=').nth(1).map(String::from);
            i += 1;
            cmd_start = i;
        } else if a == "-p" || a == "--plain" {
            plain = true;
            i += 1;
            cmd_start = i;
        } else if a == "-h" || a == "--help" {
            print_help();
            return;
        } else {
            break;
        }
    }

    if dir_override.is_none() {
        dir_override = env::var("AMARANTHINE_DIR").ok();
    }

    let dir = config::resolve_dir(dir_override);
    let cmd = &args[cmd_start..];

    let result: Result<String, String> = match cmd.first().map(|s| s.as_str()) {
        Some("store") if cmd.len() >= 3 => {
            let tags = parse_flag_str(cmd, "--tags");
            let text_parts: Vec<&str> = cmd[2..].iter()
                .filter(|a| *a != "--tags")
                .filter(|a| {
                    let prev = cmd.iter().position(|x| x == *a);
                    prev.map_or(true, |i| i == 0 || cmd[i - 1] != "--tags")
                })
                .map(|s| s.as_str()).collect();
            let text = text_parts.join(" ");
            store::run_with_tags(&dir, &cmd[1], &text, tags.as_deref())
        }
        Some("store") if cmd.len() == 2 => store::run(&dir, &cmd[1], "-"),
        Some("store") => Err("usage: store <topic> <text|-> [--tags t1,t2]".into()),
        Some("append") if cmd.len() >= 3 => store::append(&dir, &cmd[1], &cmd[2..].join(" ")),
        Some("append") if cmd.len() == 2 => store::append(&dir, &cmd[1], "-"),
        Some("append") => Err("usage: append <topic> <text|-> (adds to last entry)".into()),
        Some("search") if cmd.len() >= 2 => {
            let brief = cmd.iter().any(|a| a == "--brief" || a == "-b");
            let count_only = cmd.iter().any(|a| a == "--count" || a == "-c");
            let topics_only = cmd.iter().any(|a| a == "--topics" || a == "-t");
            let limit: Option<usize> = parse_flag_value(cmd, "--limit");
            let after = parse_flag_str(cmd, "--after").and_then(|s| time::parse_date_days(&s));
            let before = parse_flag_str(cmd, "--before").and_then(|s| time::parse_date_days(&s));
            let tag = parse_flag_str(cmd, "--tag");
            let filter = search::Filter { after, before, tag };
            let skip = ["--brief", "-b", "--count", "-c", "--topics", "-t",
                        "--limit", "--after", "--before", "--tag"];
            let query_parts: Vec<&str> = cmd[1..].iter()
                .filter(|a| !skip.contains(&a.as_str()))
                .filter(|a| {
                    let prev = cmd.iter().position(|x| x == *a);
                    prev.map_or(true, |i| {
                        i == 0 || !["--limit", "--after", "--before", "--tag"].contains(&cmd[i - 1].as_str())
                    })
                })
                .map(|s| s.as_str()).collect();
            let q = query_parts.join(" ");
            if count_only {
                search::count(&dir, &q, &filter)
            } else if topics_only {
                search::run_topics(&dir, &q, &filter)
            } else if brief {
                search::run_brief(&dir, &q, limit, &filter)
            } else {
                search::run(&dir, &q, plain, limit, &filter)
            }
        }
        Some("search") => Err("usage: search <query> [--brief|--count|--topics] [--limit N] [--after DATE] [--before DATE] [--tag TAG]".into()),
        Some("context") => {
            let brief = cmd.iter().any(|a| a == "--brief" || a == "-b");
            let query_parts: Vec<&str> = cmd[1..].iter()
                .filter(|a| *a != "--brief" && *a != "-b")
                .map(|s| s.as_str()).collect();
            let q = if query_parts.is_empty() { None } else { Some(query_parts.join(" ")) };
            if brief {
                context::run_brief(&dir, q.as_deref(), plain)
            } else {
                context::run(&dir, q.as_deref(), plain)
            }
        }
        Some("delete") if cmd.len() >= 2 => {
            let last = cmd.iter().any(|a| a == "--last");
            let all = cmd.iter().any(|a| a == "--all");
            let match_str = parse_flag_str(cmd, "--match");
            delete::run(&dir, &cmd[1], last, all, match_str.as_deref())
        }
        Some("delete") => Err("usage: delete <topic> [--last|--all|--match <str>]".into()),
        Some("edit") if cmd.len() >= 4 => {
            let match_str = parse_flag_str(cmd, "--match");
            match match_str {
                Some(needle) => {
                    let mi = cmd.iter().position(|a| a == "--match").unwrap();
                    let text_parts: Vec<&str> = cmd.iter().enumerate()
                        .filter(|(i, a)| *i != 0 && *i != 1 && *i != mi && *i != mi + 1 && !a.is_empty())
                        .map(|(_, a)| a.as_str())
                        .collect();
                    if text_parts.is_empty() {
                        Err("usage: edit <topic> --match <substring> <new text>".into())
                    } else {
                        edit::run(&dir, &cmd[1], &needle, &text_parts.join(" "))
                    }
                }
                None => Err("usage: edit <topic> --match <substring> <new text>".into()),
            }
        }
        Some("edit") => Err("usage: edit <topic> --match <substring> <new text>".into()),
        Some("index") => index::run(&dir),
        Some("recent") => {
            let days = cmd.get(1).and_then(|s| s.parse().ok()).unwrap_or(7u64);
            topics::recent(&dir, days, plain)
        }
        Some("topics") => topics::list(&dir),
        Some("prune") => {
            let stale = parse_flag_value(cmd, "--stale").unwrap_or(30u64);
            prune::run(&dir, stale, plain)
        }
        Some("digest") => digest::run(&dir),
        Some("serve") => {
            let d = if cmd.len() >= 3 && (cmd[1] == "--dir" || cmd[1] == "-d") {
                std::path::PathBuf::from(&cmd[2])
            } else { dir.clone() };
            mcp::run(&d).map(|()| String::new())
        }
        Some("install") => install::run(&dir).map(|()| String::new()),
        Some("init") => config::init(cmd.get(1).map(|s| s.as_str())).map(|()| String::new()),
        Some("help") | None => { print_help(); Ok(String::new()) }
        Some(c) => Err(format!("unknown command: {c}")),
    };

    match result {
        Ok(msg) => { if !msg.is_empty() { print!("{msg}"); } }
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    }
}

fn parse_flag_value<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
}

fn parse_flag_str(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn print_help() {
    print!(concat!(
        "amaranthine — persistent knowledge base for AI dev\n\n",
        "USAGE: amaranthine [OPTIONS] <COMMAND>\n\n",
        "COMMANDS:\n",
        "  store <topic> <text|-> [--tags t1,t2]  Store entry with optional tags\n",
        "  append <topic> <text|->      Add to last entry (no new timestamp)\n",
        "  search <query> [FLAGS]       Search entries\n",
        "    --brief, -b                Quick results (topic + first line)\n",
        "    --count, -c                Just count matches\n",
        "    --topics, -t               Which topics matched + hit count\n",
        "    --limit N                  Cap results\n",
        "    --after YYYY-MM-DD         Entries on or after date\n",
        "    --before YYYY-MM-DD        Entries on or before date\n",
        "    --tag TAG                  Filter to entries with tag\n",
        "  context [query] [--brief]    Session briefing (--brief: topics only)\n",
        "  delete <topic> --last|--all|--match <str>  Remove entries\n",
        "  edit <topic> --match <str> <text>           Update matching entry\n",
        "  index                        Generate topic manifest\n",
        "  recent [days]                Entries from last N days (default: 7)\n",
        "  topics                       List topics with counts\n",
        "  prune [--stale N]            Flag stale topics (default: 30 days)\n",
        "  digest                       Compact summary for MEMORY.md\n",
        "  serve                        MCP server over stdio\n",
        "  install                      Add to Claude Code settings\n",
        "  init [path]                  Initialize memory directory\n\n",
        "OPTIONS:\n",
        "  -d, --dir <DIR>   Memory directory (or AMARANTHINE_DIR)\n",
        "  -p, --plain       Strip colors for programmatic use\n",
    ));
}
