//! Claude Code hook handlers: ambient context, build reminders, session management.

use std::io::Read;
use std::path::Path;
use crate::json::Value;

pub fn run(hook_type: &str, dir: &Path) -> Result<String, String> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok();
    let val = if input.trim().is_empty() { Value::Null } else {
        crate::json::parse(input.trim()).unwrap_or(Value::Null)
    };
    match hook_type {
        "ambient" => ambient(&val, dir),
        "post-build" => post_build(&val),
        "stop" => stop(),
        "subagent-start" => subagent_start(dir),
        _ => Err(format!("unknown hook type: {hook_type}")),
    }
}

fn hook_output(context: &str) -> String {
    Value::Obj(vec![
        ("hookSpecificOutput".into(), Value::Obj(vec![
            ("additionalContext".into(), Value::Str(context.into())),
        ])),
    ]).to_string()
}

/// PreToolUse: inject amaranthine entries relevant to the file being accessed.
fn ambient(input: &Value, dir: &Path) -> Result<String, String> {
    let tool = input.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    match tool {
        "Read" | "Edit" | "Write" | "Glob" | "Grep" | "NotebookEdit" => {}
        _ => return Ok(String::new()),
    }
    let path = input.get("tool_input")
        .and_then(|ti| ti.get("file_path").or_else(|| ti.get("path")))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if path.is_empty() { return Ok(String::new()); }

    let stem = std::path::Path::new(path)
        .file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.len() < 3 { return Ok(String::new()); }

    let data = match std::fs::read(dir.join("index.bin")) {
        Ok(d) => d,
        Err(_) => return Ok(String::new()),
    };
    let results = crate::binquery::search(&data, stem, 5).unwrap_or_default();
    let has_results = !results.is_empty() && !results.starts_with("0 match");

    // Also surface structural coupling entries that reference this file
    let structural_query = format!("structural {stem}");
    let structural = crate::binquery::search(&data, &structural_query, 3).unwrap_or_default();
    let has_structural = !structural.is_empty() && !structural.starts_with("0 match");

    // Edit-aware: detect removed/renamed symbols and surface coupling
    let mut refactor_context = String::new();
    if tool == "Edit" {
        let ti = input.get("tool_input");
        let old = ti.and_then(|t| t.get("old_string")).and_then(|v| v.as_str()).unwrap_or("");
        let new_str = ti.and_then(|t| t.get("new_string")).and_then(|v| v.as_str()).unwrap_or("");
        if old.len() >= 8 {
            let extract = |s: &str| -> std::collections::HashSet<String> {
                s.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .filter(|w| w.len() >= 4 && w.bytes().any(|b| b.is_ascii_alphabetic()))
                    .map(|w| w.to_lowercase())
                    .collect()
            };
            let old_tokens: std::collections::HashSet<String> = extract(old)
                .into_iter().filter(|t| t != stem).collect();
            let new_tokens: std::collections::HashSet<String> = extract(new_str);
            let mut removed: Vec<&str> = old_tokens.iter()
                .filter(|t| !new_tokens.contains(*t))
                .map(|s| s.as_str()).collect();
            removed.sort();
            removed.truncate(3);
            if !removed.is_empty() {
                let syms = removed.join(", ");
                refactor_context.push_str(
                    &format!("\nREFACTOR IMPACT (symbols modified: {syms}):\n"));
                for sym in &removed {
                    let hits = crate::binquery::search(&data, sym, 3).unwrap_or_default();
                    if !hits.is_empty() && !hits.starts_with("0 match") {
                        refactor_context.push_str(&hits);
                    }
                }
            }
        }
    }

    let has_refactor = !refactor_context.is_empty();
    if !has_results && !has_structural && !has_refactor { return Ok(String::new()); }

    let mut out = String::new();
    if has_results { out.push_str(&format!("amaranthine entries for {stem}:\n{results}")); }
    if has_structural {
        if has_results { out.push_str("\n---\n"); }
        out.push_str(&format!("structural coupling:\n{structural}"));
    }
    if has_refactor {
        if has_results || has_structural { out.push_str("\n---\n"); }
        out.push_str(&refactor_context);
    }
    Ok(hook_output(&out))
}

/// PostToolUse(Bash): after build commands, remind to store results.
fn post_build(input: &Value) -> Result<String, String> {
    let cmd = input.get("tool_input")
        .and_then(|ti| ti.get("command"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let is_build = (cmd.contains("xcodebuild") && cmd.contains("build"))
        || cmd.contains("cargo build") || cmd.contains("swift build")
        || cmd.starts_with("swiftc ");
    if !is_build { return Ok(String::new()); }
    Ok(hook_output("BUILD COMPLETED. If the build failed with a non-obvious error, \
        store the root cause in amaranthine (topic: build-gotchas). \
        If it succeeded after fixing an issue, store what fixed it."))
}

/// Stop: remind to store findings before conversation ends.
fn stop() -> Result<String, String> {
    let stamp = "/tmp/amaranthine-hook-stop.last";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(content) = std::fs::read_to_string(stamp) {
        if let Ok(last) = content.trim().parse::<u64>() {
            if now.saturating_sub(last) < 120 { return Ok(String::new()); }
        }
    }
    std::fs::write(stamp, now.to_string()).ok();
    Ok(hook_output("STOPPING: Store any non-obvious findings in amaranthine before ending."))
}

/// SubagentStart: inject dynamic topic list from index.
fn subagent_start(dir: &Path) -> Result<String, String> {
    let data = match std::fs::read(dir.join("index.bin")) {
        Ok(d) => d,
        Err(_) => return Ok(hook_output(
            "AMARANTHINE KNOWLEDGE STORE: You have access to amaranthine MCP tools. \
             Search before starting work.")),
    };
    let topics = crate::binquery::topic_table(&data).unwrap_or_default();
    let mut list: Vec<String> = topics.iter()
        .map(|(_, name, count)| format!("{name} ({count})"))
        .collect();
    list.sort();
    Ok(hook_output(&format!(
        "AMARANTHINE KNOWLEDGE STORE: You have access to amaranthine MCP tools. \
         BEFORE starting work, call mcp__amaranthine__search with keywords \
         relevant to your task. Topics: {}", list.join(", "))))
}
