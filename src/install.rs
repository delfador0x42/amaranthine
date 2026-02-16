use std::fs;
use std::path::{Path, PathBuf};

pub fn run(_dir: &Path) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_s = exe.to_string_lossy();

    let global_dir = PathBuf::from(&home).join(".amaranthine");
    if !global_dir.exists() {
        fs::create_dir_all(&global_dir).map_err(|e| e.to_string())?;
        println!("created ~/.amaranthine/");
    }

    // MCP servers live in ~/.claude.json (NOT ~/.claude/mcp.json or settings.json)
    let claude_json = PathBuf::from(&home).join(".claude.json");
    update_claude_json(&claude_json, &exe_s)?;

    let claude_md = PathBuf::from(&home).join(".claude/CLAUDE.md");
    update_claude_md(&claude_md, &exe_s)?;

    println!("\namaranthine installed. restart claude code to pick up MCP server.");
    println!("all knowledge lives in ~/.amaranthine/");
    Ok(())
}

fn update_claude_json(path: &Path, exe: &str) -> Result<(), String> {
    let content = if path.exists() {
        fs::read_to_string(path).map_err(|e| e.to_string())?
    } else {
        "{}".into()
    };

    let mut config = crate::json::parse(&content)
        .unwrap_or(crate::json::Value::Obj(Vec::new()));

    if config.get("mcpServers")
        .and_then(|s| s.get("amaranthine"))
        .is_some()
    {
        println!(".claude.json: amaranthine already configured");
        return Ok(());
    }

    use crate::json::Value;
    if config.get("mcpServers").is_none() {
        config.set("mcpServers", Value::Obj(Vec::new()));
    }
    let server = Value::Obj(vec![
        ("command".into(), Value::Str(exe.into())),
        ("args".into(), Value::Arr(vec![Value::Str("serve".into())])),
    ]);
    config.get_mut("mcpServers").unwrap().set("amaranthine", server);

    fs::write(path, config.pretty()).map_err(|e| e.to_string())?;
    println!(".claude.json: added amaranthine MCP server");
    Ok(())
}

fn update_claude_md(path: &Path, _exe: &str) -> Result<(), String> {
    if !path.exists() {
        println!("CLAUDE.md: not found, skipping");
        return Ok(());
    }

    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if content.contains("amaranthine") {
        println!("CLAUDE.md: already references amaranthine");
        return Ok(());
    }

    let section = concat!(
        "\n## Memory \u{2014} amaranthine\n",
        "Cross-session knowledge store. Always use **MCP tools** (prefixed `amaranthine__`).\n\n",
        "**MANDATORY \u{2014} every session, every task:**\n",
        "1. `context(brief: \"true\")` at session start \u{2014} load all topic knowledge\n",
        "2. `search(query)` BEFORE starting any feature/fix \u{2014} check for prior learnings\n",
        "3. `store(topic, text)` DURING work \u{2014} atomic facts as you discover them\n\n",
        "**Searching:** `search(query)` | `search_brief` (fast) | `search_count` (fastest)\n",
        "**Writing:** `store(topic, text, tags?)` | `append(topic, text)` | `delete_entry`\n\n",
        "**Discipline:**\n",
        "- Store each non-obvious finding IMMEDIATELY \u{2014} don\u{2019}t batch them\n",
        "- Small atomic entries > big summaries. Searchable > comprehensive.\n",
        "- Delete wrong info immediately.\n",
    );

    fs::write(path, format!("{content}{section}")).map_err(|e| e.to_string())?;
    println!("CLAUDE.md: added amaranthine workflow section");
    Ok(())
}
