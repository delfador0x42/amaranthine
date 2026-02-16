use crate::time::LocalTime;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

pub fn run(dir: &Path, topic: &str, text: &str) -> Result<String, String> {
    crate::config::ensure_dir(dir)?;
    let text = read_text(text)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    // Check for duplicates — warn but still store
    let dupe_warn = check_dupe(&filepath, &text);

    let timestamp = LocalTime::now();
    let is_new = !filepath.exists();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&filepath)
        .map_err(|e| format!("can't open {}: {e}", filepath.display()))?;

    if is_new {
        writeln!(file, "# {topic}\n").map_err(|e| e.to_string())?;
    }
    writeln!(file, "## {timestamp}").map_err(|e| e.to_string())?;
    writeln!(file, "{text}\n").map_err(|e| e.to_string())?;

    let count = count_entries(&filepath);
    let mut msg = format!("stored in {filename}.md ({count} entries)");
    if let Some(warn) = dupe_warn {
        msg.push_str(&format!("\n  warning: {warn}"));
    }
    Ok(msg)
}

/// Append text to the LAST entry in a topic (no new timestamp)
pub fn append(dir: &Path, topic: &str, text: &str) -> Result<String, String> {
    let text = read_text(text)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found — use 'store' first"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    // Find the last entry and append to it
    let sections = crate::delete::split_sections(&content);
    if sections.is_empty() {
        return Err("no entries to append to".into());
    }

    // Append by adding text before the trailing newline
    let trimmed = content.trim_end();
    let result = format!("{trimmed}\n{text}\n\n");
    fs::write(&filepath, &result).map_err(|e| e.to_string())?;
    Ok(format!("appended to last entry in {filename}.md"))
}

fn read_text(text: &str) -> Result<String, String> {
    if text == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).map_err(|e| e.to_string())?;
        let trimmed = buf.trim_end();
        if trimmed.is_empty() {
            return Err("empty stdin".into());
        }
        Ok(trimmed.to_string())
    } else {
        Ok(text.to_string())
    }
}

/// Check if similar content already exists in the topic file
fn check_dupe(filepath: &Path, new_text: &str) -> Option<String> {
    let content = fs::read_to_string(filepath).ok()?;
    let new_lower = new_text.to_lowercase();
    // Extract significant words (4+ chars) from new text
    let words: Vec<&str> = new_lower.split_whitespace()
        .filter(|w| w.len() >= 4)
        .collect();
    if words.len() < 3 { return None; }

    // Check each existing entry for word overlap
    let sections = crate::delete::split_sections(&content);
    for (_, body) in &sections {
        let body_lower = body.to_lowercase();
        let matches = words.iter().filter(|w| body_lower.contains(**w)).count();
        let ratio = matches as f64 / words.len() as f64;
        if ratio > 0.6 {
            return Some("similar content may already exist in this topic".into());
        }
    }
    None
}

fn count_entries(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| l.starts_with("## ")).count())
        .unwrap_or(0)
}
