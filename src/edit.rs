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
    crate::config::atomic_write(&filepath, &result)?;
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
    crate::config::atomic_write(&filepath, &result)?;
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
    crate::config::atomic_write(&filepath, &result)?;
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
    crate::config::atomic_write(&filepath, &result)?;
    Ok(format!("appended to entry [{idx}] in {filename}.md"))
}

/// Add a [modified: timestamp] marker to updated text.
fn add_modified_marker(text: &str) -> String {
    let now = crate::time::LocalTime::now();
    format!("[modified: {now}]\n{text}")
}

/// Rename a topic file. Preserves all entries.
pub fn rename_topic(dir: &Path, old_name: &str, new_name: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let old_fn = crate::config::sanitize_topic(old_name);
    let new_fn = crate::config::sanitize_topic(new_name);
    let old_path = dir.join(format!("{old_fn}.md"));
    let new_path = dir.join(format!("{new_fn}.md"));

    if !old_path.exists() {
        return Err(format!("{old_fn}.md not found"));
    }
    if new_path.exists() {
        return Err(format!("{new_fn}.md already exists"));
    }

    // Update the title line inside the file
    let content = fs::read_to_string(&old_path).map_err(|e| e.to_string())?;
    let updated = if content.starts_with(&format!("# {old_name}")) {
        content.replacen(&format!("# {old_name}"), &format!("# {new_name}"), 1)
    } else {
        content
    };
    crate::config::atomic_write(&new_path, &updated)?;
    fs::remove_file(&old_path).map_err(|e| format!("remove old: {e}"))?;
    Ok(format!("renamed {old_fn}.md → {new_fn}.md"))
}

/// Add or remove tags on an existing entry by index or match.
pub fn tag_entry(
    dir: &Path, topic: &str,
    idx: Option<usize>, needle: Option<&str>,
    add: Option<&str>, remove: Option<&str>,
) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));
    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = crate::delete::split_sections(&content);

    // Find the target entry
    let target_idx = if let Some(i) = idx {
        if i >= sections.len() {
            return Err(format!("index {i} out of range (0-{})", sections.len().saturating_sub(1)));
        }
        i
    } else if let Some(n) = needle {
        let lower = n.to_lowercase();
        sections.iter().position(|(_, body)| body.to_lowercase().contains(&lower))
            .ok_or_else(|| format!("no entry matching \"{n}\""))?
    } else {
        return Err("provide index or match_str".into());
    };

    let (_, body) = &sections[target_idx];

    // Parse existing tags
    let mut tags: Vec<String> = Vec::new();
    let mut body_without_tags = String::new();
    for line in body.lines() {
        if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
            for t in inner.split(',') {
                let t = t.trim().to_lowercase();
                if !t.is_empty() { tags.push(t); }
            }
        } else {
            body_without_tags.push_str(line);
            body_without_tags.push('\n');
        }
    }

    // Add new tags
    if let Some(add_str) = add {
        for t in add_str.split(',') {
            let t = t.trim().to_lowercase();
            if !t.is_empty() && !tags.contains(&t) { tags.push(t); }
        }
    }
    // Remove tags
    if let Some(rm_str) = remove {
        let rm: Vec<String> = rm_str.split(',').map(|t| t.trim().to_lowercase()).collect();
        tags.retain(|t| !rm.contains(t));
    }

    tags.sort();
    tags.dedup();

    // Rebuild body: tags line first (if any), then content
    let new_body = if tags.is_empty() {
        body_without_tags.trim().to_string()
    } else {
        format!("[tags: {}]\n{}", tags.join(", "), body_without_tags.trim())
    };

    let result = crate::delete::rebuild_file(&content, &sections, None, Some((target_idx, &new_body)));
    crate::config::atomic_write(&filepath, &result)?;
    Ok(format!("tags updated on entry [{target_idx}] in {filename}.md: [{}]", tags.join(", ")))
}
