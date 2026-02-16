use std::fs;
use std::path::Path;

/// Replace the content of the first entry matching `needle` with `new_text`.
/// Keeps the original timestamp header. Adds [modified] marker.
pub fn run(dir: &Path, topic: &str, needle: &str, new_text: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = crate::delete::split_sections(&content);
    let lower = needle.to_lowercase();

    let idx = sections.iter().position(|(_, body)| body.to_lowercase().contains(&lower));
    let idx = match idx {
        Some(i) => i,
        None => return Err(format!("no entry matching \"{needle}\"")),
    };

    let body_with_marker = add_modified_marker(new_text);
    let result = crate::delete::rebuild_file(&content, &sections, None, Some((idx, &body_with_marker)));
    fs::write(&filepath, &result).map_err(|e| e.to_string())?;
    Ok(format!("updated entry matching \"{needle}\" in {filename}.md"))
}

/// Replace entry by 0-based index. Adds [modified] marker.
pub fn run_by_index(dir: &Path, topic: &str, idx: usize, new_text: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = crate::delete::split_sections(&content);

    if idx >= sections.len() {
        return Err(format!("index {idx} out of range (topic has {} entries, 0-{})",
            sections.len(), sections.len().saturating_sub(1)));
    }

    let body_with_marker = add_modified_marker(new_text);
    let result = crate::delete::rebuild_file(&content, &sections, None, Some((idx, &body_with_marker)));
    fs::write(&filepath, &result).map_err(|e| e.to_string())?;
    Ok(format!("updated entry [{idx}] in {filename}.md"))
}

/// Append text to the first entry matching `needle`. Keeps timestamp and existing body.
pub fn append(dir: &Path, topic: &str, needle: &str, extra: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = crate::delete::split_sections(&content);
    let lower = needle.to_lowercase();

    let idx = sections.iter().position(|(_, body)| body.to_lowercase().contains(&lower));
    let idx = match idx {
        Some(i) => i,
        None => return Err(format!("no entry matching \"{needle}\"")),
    };

    // Concatenate existing body (trimmed) with new text
    let existing = sections[idx].1.trim();
    let combined = format!("{existing}\n{extra}");
    let result = crate::delete::rebuild_file(&content, &sections, None, Some((idx, &combined)));
    fs::write(&filepath, &result).map_err(|e| e.to_string())?;
    Ok(format!("appended to entry matching \"{needle}\" in {filename}.md"))
}

/// Append text to entry by 0-based index.
pub fn append_by_index(dir: &Path, topic: &str, idx: usize, extra: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = crate::delete::split_sections(&content);

    if idx >= sections.len() {
        return Err(format!("index {idx} out of range (topic has {} entries, 0-{})",
            sections.len(), sections.len().saturating_sub(1)));
    }

    let existing = sections[idx].1.trim();
    let combined = format!("{existing}\n{extra}");
    let result = crate::delete::rebuild_file(&content, &sections, None, Some((idx, &combined)));
    fs::write(&filepath, &result).map_err(|e| e.to_string())?;
    Ok(format!("appended to entry [{idx}] in {filename}.md"))
}

/// Add a [modified: timestamp] marker to updated text.
fn add_modified_marker(text: &str) -> String {
    let now = crate::time::LocalTime::now();
    format!("[modified: {now}]\n{text}")
}
