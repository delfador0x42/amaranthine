use crate::time::LocalTime;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub fn run(dir: &Path, topic: &str, text: &str) -> Result<String, String> {
    run_full(dir, topic, text, None, false)
}

pub fn run_with_tags(dir: &Path, topic: &str, text: &str, tags: Option<&str>) -> Result<String, String> {
    run_full(dir, topic, text, tags, false)
}

pub fn run_full(dir: &Path, topic: &str, text: &str, tags: Option<&str>, force: bool) -> Result<String, String> {
    crate::config::ensure_dir(dir)?;
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let text = read_text(text)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    // Check for duplicates — warn/block if high overlap (skip with force)
    if !force {
        if let Some(existing) = check_dupe(&filepath, &text) {
            return Err(format!("blocked: similar entry already exists in {filename}.md\n  existing: {existing}\nUse update_entry or append_entry to modify it, or pass force=true to override."));
        }
    }
    // Suggest existing topics if this is a new topic
    let topic_hint = if !filepath.exists() { suggest_topic(dir, &filename) } else { None };

    let timestamp = LocalTime::now();
    let is_new = !filepath.exists();

    // Normalize tags: lowercase, trim, dedupe
    let cleaned_tags = tags.map(|t| normalize_tags(t));
    let tag_warn = tags.and_then(|t| check_similar_tags(dir, t));

    // Build new content: read existing + append new entry
    let mut content = if is_new {
        format!("# {topic}\n\n")
    } else {
        fs::read_to_string(&filepath)
            .map_err(|e| format!("can't read {}: {e}", filepath.display()))?
    };

    content.push_str(&format!("## {timestamp}\n"));
    if let Some(ref tags) = cleaned_tags {
        if !tags.is_empty() {
            content.push_str(&format!("[tags: {tags}]\n"));
        }
    }
    content.push_str(&format!("{text}\n\n"));

    crate::config::atomic_write(&filepath, &content)?;

    let count = count_entries(&filepath);

    // Echo back what was stored (full text, indented)
    let echo_text = text.lines()
        .map(|l| format!("  > {l}"))
        .collect::<Vec<_>>().join("\n");
    let tag_echo = cleaned_tags.as_deref().filter(|t| !t.is_empty())
        .map(|t| format!(" [tags: {t}]"))
        .unwrap_or_default();

    let mut msg = format!("stored in {filename}.md ({count} entries)\n  @ {timestamp}{tag_echo}\n{echo_text}");
    if let Some(hint) = topic_hint {
        msg.push_str(&format!("\n  note: {hint}"));
    }
    if let Some(tw) = tag_warn {
        msg.push_str(&format!("\n  tag note: {tw}"));
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
    let _lock = crate::lock::FileLock::acquire(dir)?;
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
    crate::config::atomic_write(&filepath, &result)?;
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

/// Normalize tags: lowercase, trim, singularize, dedupe, sort.
fn normalize_tags(raw: &str) -> String {
    let mut tags: Vec<String> = raw.split(',')
        .map(|t| singularize(t.trim()).to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    tags.sort();
    tags.dedup();
    tags.join(", ")
}

/// Simple English singularization for tag normalization.
fn singularize(s: &str) -> String {
    let s = s.trim();
    if s.len() <= 2 { return s.to_string(); }
    // "ies" → "y" (e.g. "entries" → "entry")
    if s.ends_with("ies") && s.len() > 4 {
        return format!("{}y", &s[..s.len() - 3]);
    }
    // "sses" → "ss" (e.g. "classes")
    if s.ends_with("sses") {
        return s[..s.len() - 2].to_string();
    }
    // trailing "s" but not "ss" or "us" or "is"
    if s.ends_with('s') && !s.ends_with("ss") && !s.ends_with("us") && !s.ends_with("is") {
        return s[..s.len() - 1].to_string();
    }
    s.to_string()
}

/// Check if provided tags are close to existing ones (catch "bugs" vs "bug").
fn check_similar_tags(dir: &Path, raw: &str) -> Option<String> {
    let existing = collect_tag_vocab(dir)?;
    let new_tags: Vec<String> = raw.split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    let mut warnings = Vec::new();
    for tag in &new_tags {
        if existing.contains_key(tag) { continue; }
        // Check for near-matches: one is prefix of other, or differ by trailing 's'
        for (existing_tag, count) in &existing {
            let similar = tag.starts_with(existing_tag.as_str())
                || existing_tag.starts_with(tag.as_str())
                || (tag.ends_with('s') && &tag[..tag.len()-1] == existing_tag.as_str())
                || (existing_tag.ends_with('s') && &existing_tag[..existing_tag.len()-1] == tag.as_str());
            if similar {
                warnings.push(format!("'{tag}' — did you mean '{existing_tag}' ({count} uses)?"));
                break;
            }
        }
    }
    if warnings.is_empty() { None } else { Some(warnings.join("; ")) }
}

/// Collect all tags with their usage counts.
fn collect_tag_vocab(dir: &Path) -> Option<std::collections::BTreeMap<String, usize>> {
    let files = crate::config::list_topic_files(dir).ok()?;
    let mut tags = std::collections::BTreeMap::new();
    for path in &files {
        let content = fs::read_to_string(path).ok()?;
        for line in content.lines() {
            if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                for tag in inner.split(',') {
                    let t = tag.trim().to_lowercase();
                    if !t.is_empty() { *tags.entry(t).or_insert(0) += 1; }
                }
            }
        }
    }
    Some(tags)
}

/// Common words to exclude from dupe detection (they inflate overlap on topic-related entries).
const STOP_WORDS: &[&str] = &[
    "that", "this", "with", "from", "have", "been", "were", "will", "when",
    "which", "their", "there", "about", "would", "could", "should", "into",
    "also", "each", "does", "just", "more", "than", "then", "them", "some",
    "only", "other", "very", "after", "before", "most", "same", "both",
    "used", "uses", "using", "need", "added", "file", "path", "type", "name",
];

/// Check if similar content already exists. Returns existing entry text if >85% unique-word overlap.
/// Uses deduplicated significant words (≥5 chars, no stop words) for better precision.
fn check_dupe(filepath: &Path, new_text: &str) -> Option<String> {
    let content = fs::read_to_string(filepath).ok()?;
    let new_lower = new_text.to_lowercase();
    let mut seen = std::collections::HashSet::new();
    let words: Vec<&str> = new_lower.split_whitespace()
        .filter(|w| w.len() >= 5)
        .filter(|w| !STOP_WORDS.contains(w))
        .filter(|w| seen.insert(*w))
        .collect();
    if words.len() < 4 { return None; }

    let sections = crate::delete::split_sections(&content);
    for (header, body) in &sections {
        let body_lower = body.to_lowercase();
        let matches = words.iter().filter(|w| body_lower.contains(**w)).count();
        let ratio = matches as f64 / words.len() as f64;
        if ratio > 0.85 {
            let preview = body.trim().lines().next().unwrap_or("").trim();
            let short = if preview.len() > 100 {
                let mut end = 100;
                while end > 0 && !preview.is_char_boundary(end) { end -= 1; }
                format!("{}...", &preview[..end])
            } else { preview.to_string() };
            return Some(format!("[{header}] {short}"));
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
    let all_tags = collect_tag_vocab(dir)?;
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
