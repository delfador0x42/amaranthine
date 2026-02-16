use crate::time::LocalTime;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

pub fn run(dir: &Path, topic: &str, text: &str) -> Result<String, String> {
    run_with_tags(dir, topic, text, None)
}

pub fn run_with_tags(dir: &Path, topic: &str, text: &str, tags: Option<&str>) -> Result<String, String> {
    crate::config::ensure_dir(dir)?;
    let text = read_text(text)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    // Check for duplicates — warn but still store
    let dupe_warn = check_dupe(&filepath, &text);
    // Suggest existing topics if this is a new topic
    let topic_hint = if !filepath.exists() { suggest_topic(dir, &filename) } else { None };

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
    if let Some(tags) = tags {
        let cleaned: Vec<&str> = tags.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
        if !cleaned.is_empty() {
            writeln!(file, "[tags: {}]", cleaned.join(", ")).map_err(|e| e.to_string())?;
        }
    }
    writeln!(file, "{text}\n").map_err(|e| e.to_string())?;

    let count = count_entries(&filepath);
    let mut msg = format!("stored in {filename}.md ({count} entries)");
    if let Some(warn) = dupe_warn {
        msg.push_str(&format!("\n  warning: {warn}"));
    }
    if let Some(hint) = topic_hint {
        msg.push_str(&format!("\n  note: {hint}"));
    }
    // Suggest tags from existing vocabulary if none provided
    if tags.is_none() {
        if let Some(suggestions) = suggest_tags(dir, &text) {
            msg.push_str(&format!("\n  suggested tags: {suggestions}"));
        }
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

/// When creating a new topic, check for similar existing topic names.
fn suggest_topic(dir: &Path, new_name: &str) -> Option<String> {
    let files = crate::config::list_topic_files(dir).ok()?;
    let parts: Vec<&str> = new_name.split('-').collect();
    let mut similar: Vec<String> = Vec::new();

    for path in &files {
        let name = path.file_stem()?.to_string_lossy().to_string();
        // Check if any part of the new topic name appears in existing topics
        let shared = parts.iter().filter(|p| p.len() >= 3 && name.contains(**p)).count();
        if shared > 0 && name != new_name {
            similar.push(name);
        }
    }
    if similar.is_empty() { return None; }
    Some(format!("new topic created. similar existing topics: {}", similar.join(", ")))
}

/// Suggest tags from existing vocabulary based on text content.
fn suggest_tags(dir: &Path, text: &str) -> Option<String> {
    let files = crate::config::list_topic_files(dir).ok()?;
    let mut all_tags: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for path in &files {
        let content = fs::read_to_string(path).ok()?;
        for line in content.lines() {
            if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                for tag in inner.split(',') {
                    let t = tag.trim().to_lowercase();
                    if !t.is_empty() { *all_tags.entry(t).or_insert(0) += 1; }
                }
            }
        }
    }
    if all_tags.is_empty() { return None; }

    let text_lower = text.to_lowercase();
    let text_words: Vec<&str> = text_lower.split_whitespace().collect();
    let mut matched: Vec<(&str, usize)> = Vec::new();

    for (tag, count) in &all_tags {
        // Match if tag appears as a word or substring in text
        if text_words.iter().any(|w| w.contains(tag.as_str())) || text_lower.contains(tag.as_str()) {
            matched.push((tag.as_str(), *count));
        }
    }
    if matched.is_empty() { return None; }

    // Sort by frequency (most used first), take top 5
    matched.sort_by(|a, b| b.1.cmp(&a.1));
    matched.truncate(5);
    let tags: Vec<&str> = matched.iter().map(|(t, _)| *t).collect();
    Some(tags.join(", "))
}

fn count_entries(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| l.starts_with("## ")).count())
        .unwrap_or(0)
}
