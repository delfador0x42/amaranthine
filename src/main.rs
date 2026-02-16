mod config;
mod context;
mod delete;
mod digest;
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

    let result = match cmd.first().map(|s| s.as_str()) {
        Some("store") if cmd.len() >= 3 => store::run(&dir, &cmd[1], &cmd[2..].join(" ")),
        Some("store") if cmd.len() == 2 => store::run(&dir, &cmd[1], "-"),
        Some("store") => Err("usage: store <topic> <text|-> (- reads stdin)".into()),
        Some("search") if cmd.len() >= 2 => search::run(&dir, &cmd[1..].join(" "), plain),
        Some("search") => Err("usage: search <query>".into()),
        Some("context") => {
            let q = if cmd.len() >= 2 { Some(cmd[1..].join(" ")) } else { None };
            context::run(&dir, q.as_deref(), plain)
        }
        Some("delete") if cmd.len() >= 2 => {
            let last = cmd.iter().any(|a| a == "--last");
            let all = cmd.iter().any(|a| a == "--all");
            delete::run(&dir, &cmd[1], last, all)
        }
        Some("delete") => Err("usage: delete <topic> [--last|--all]".into()),
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
            mcp::run(&d)
        }
        Some("install") => install::run(&dir),
        Some("init") => config::init(cmd.get(1).map(|s| s.as_str())),
        Some("help") | None => { print_help(); Ok(()) }
        Some(c) => Err(format!("unknown command: {c}")),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn parse_flag_value<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
}

fn print_help() {
    print!(concat!(
        "amaranthine — persistent knowledge base for AI dev\n\n",
        "USAGE: amaranthine [OPTIONS] <COMMAND>\n\n",
        "COMMANDS:\n",
        "  store <topic> <text|->       Store entry (- reads stdin)\n",
        "  search <query>               Search all memory files\n",
        "  context [query]              Session briefing (topics + recent + search)\n",
        "  delete <topic> --last|--all  Remove entries\n",
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
