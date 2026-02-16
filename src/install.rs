use std::fs;
use std::path::{Path, PathBuf};

const INSTALL_DIR: &str = ".local/bin";
const BINARY_NAME: &str = "amaranthine";

pub fn run(_dir: &Path) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    // 1. Create ~/.amaranthine/
    let global_dir = PathBuf::from(&home).join(".amaranthine");
    if !global_dir.exists() {
        fs::create_dir_all(&global_dir).map_err(|e| e.to_string())?;
        println!("created ~/.amaranthine/");
    } else {
        println!("~/.amaranthine/ already exists");
    }

    // 2. Copy binary to ~/.local/bin/ and codesign
    let bin_dir = PathBuf::from(&home).join(INSTALL_DIR);
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let installed = bin_dir.join(BINARY_NAME);
    let installed_str = installed.to_string_lossy().to_string();

    if exe != installed {
        fs::copy(&exe, &installed)
            .map_err(|e| format!("copy to {}: {e}", installed.display()))?;
        println!("installed to {installed_str}");
    } else {
        println!("binary already at {installed_str}");
    }

    // macOS: ad-hoc codesign so taskgate doesn't kill it
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("codesign")
            .args(["-s", "-", "-f"])
            .arg(&installed)
            .output();
        match out {
            Ok(o) if o.status.success() => println!("codesigned {installed_str}"),
            Ok(o) => println!("codesign warning: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => println!("codesign skipped: {e}"),
        }
    }

    // 3. Add MCP server to ~/.claude.json
    let claude_json = PathBuf::from(&home).join(".claude.json");
    update_claude_json(&claude_json, &installed_str)?;

    // 4. Add usage instructions to ~/.claude/CLAUDE.md
    let claude_md = PathBuf::from(&home).join(".claude/CLAUDE.md");
    update_claude_md(&claude_md)?;

    println!("\namaranthine installed. restart claude code to pick up MCP server.");
    println!("knowledge lives in ~/.amaranthine/");
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

    // Remove stale config pointing to wrong path, re-add with correct path
    let needs_update = config.get("mcpServers")
        .and_then(|s| s.get("amaranthine"))
        .and_then(|a| a.get("command"))
        .and_then(|c| c.as_str())
        .map(|c| c != exe)
        .unwrap_or(true);

    if !needs_update {
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
    println!(".claude.json: configured amaranthine MCP server");
    Ok(())
}

fn update_claude_md(path: &Path) -> Result<(), String> {
    let dir = path.parent().ok_or("no parent dir")?;
    if !dir.exists() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }

    let content = if path.exists() {
        fs::read_to_string(path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    if content.contains("amaranthine") {
        println!("CLAUDE.md: already references amaranthine");
        return Ok(());
    }

    let section = concat!(
        "\n## Memory \u{2014} amaranthine\n",
        "Cross-session knowledge store via MCP tools (prefixed `amaranthine__`).\n\n",
        "**Every session:** `search(query)` before starting work.\n",
        "**During work:** `store(topic, text, tags?)` for non-obvious findings.\n\n",
        "**Search tools:** `search` (full) | `search_medium` (2-line preview) | ",
        "`search_brief` (1-line) | `search_topics` | `search_count`\n",
        "**Write tools:** `store` | `batch_store` | `append` | ",
        "`append_entry` | `update_entry` | `delete_entry`\n",
        "**Browse tools:** `context` | `topics` | `recent` | ",
        "`read_topic` | `digest` | `stats` | `list_tags`\n",
    );

    fs::write(path, format!("{content}{section}")).map_err(|e| e.to_string())?;
    println!("CLAUDE.md: added amaranthine section");
    Ok(())
}
