use std::fs;
use std::path::{Path, PathBuf};

pub fn run(_dir: &Path) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_s = exe.to_string_lossy();

    // Ensure global fallback dir exists
    let global_dir = PathBuf::from(&home).join(".amaranthine");
    if !global_dir.exists() {
        fs::create_dir_all(&global_dir).map_err(|e| e.to_string())?;
        println!("created ~/.amaranthine/ (global fallback)");
    }

    let claude_dir = PathBuf::from(&home).join(".claude");
    if !claude_dir.exists() {
        fs::create_dir_all(&claude_dir).map_err(|e| e.to_string())?;
    }

    update_settings(&claude_dir.join("settings.json"), &exe_s)?;
    update_claude_md(&claude_dir.join("CLAUDE.md"), &exe_s)?;

    println!("\namaranthine installed. restart claude code to pick up MCP server.");
    println!("all knowledge lives in ~/.amaranthine/");
    Ok(())
}

fn update_settings(path: &Path, exe: &str) -> Result<(), String> {
    let content = if path.exists() {
        fs::read_to_string(path).map_err(|e| e.to_string())?
    } else {
        "{}".into()
    };

    let mut settings = crate::json::parse(&content)
        .unwrap_or(crate::json::Value::Obj(Vec::new()));

    if settings.get("mcpServers")
        .and_then(|s| s.get("amaranthine"))
        .is_some()
    {
        println!("settings.json: amaranthine already configured");
        return Ok(());
    }

    use crate::json::Value;
    if settings.get("mcpServers").is_none() {
        settings.set("mcpServers", Value::Obj(Vec::new()));
    }
    // No --dir: walk-up resolution finds project .amaranthine/ or falls back to ~/
    let server_config = Value::Obj(vec![
        ("command".into(), Value::Str(exe.into())),
        ("args".into(), Value::Arr(vec![
            Value::Str("serve".into()),
        ])),
    ]);
    settings.get_mut("mcpServers").unwrap().set("amaranthine", server_config);

    fs::write(path, settings.pretty()).map_err(|e| e.to_string())?;
    println!("settings.json: added amaranthine MCP server");
    Ok(())
}

fn update_claude_md(path: &Path, exe: &str) -> Result<(), String> {
    if !path.exists() {
        println!("CLAUDE.md: not found, skipping (create ~/.claude/CLAUDE.md first)");
        return Ok(());
    }

    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if content.contains("amaranthine") {
        println!("CLAUDE.md: already references amaranthine");
        return Ok(());
    }

    let section = format!(concat!(
        "\n## Memory \u{2014} amaranthine\n",
        "amaranthine is available at {exe} and as MCP server.\n",
        "Use `amaranthine --plain search <query>` to search knowledge.\n",
        "Use `amaranthine --plain store <topic> <text>` to store knowledge.\n",
        "Use `amaranthine --plain context` for session orientation.\n",
    ), exe = exe);

    fs::write(path, format!("{content}{section}")).map_err(|e| e.to_string())?;
    println!("CLAUDE.md: added amaranthine section");
    Ok(())
}
