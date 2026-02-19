use std::path::Path;

/// Replace the content of the first entry matching `needle`.
pub fn run(dir: &Path, topic: &str, needle: &str, new_text: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;
    let lower = needle.to_lowercase();
    let entry = entries.iter().find(|e| e.body.to_lowercase().contains(&lower))
        .ok_or_else(|| format!("no entry matching \"{}\"", needle))?;
    let new_body = add_modified_marker(new_text);
    crate::datalog::append_entry(&log_path, topic, &new_body, entry.timestamp_min)?;
    crate::datalog::append_delete(&log_path, entry.offset)?;
    Ok(format!("updated entry matching \"{}\" in {}", needle, topic))
}

/// Replace entry by 0-based index.
pub fn run_by_index(dir: &Path, topic: &str, idx: usize, new_text: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;
    if idx >= entries.len() {
        return Err(format!("index {idx} out of range (topic has {} entries, 0-{})",
            entries.len(), entries.len().saturating_sub(1)));
    }
    let entry = &entries[idx];
    let new_body = add_modified_marker(new_text);
    crate::datalog::append_entry(&log_path, topic, &new_body, entry.timestamp_min)?;
    crate::datalog::append_delete(&log_path, entry.offset)?;
    Ok(format!("updated entry [{idx}] in {}", topic))
}

/// Append text to the first entry matching `needle`.
pub fn append(dir: &Path, topic: &str, needle: &str, extra: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;
    let lower = needle.to_lowercase();
    let entry = entries.iter().find(|e| e.body.to_lowercase().contains(&lower))
        .ok_or_else(|| format!("no entry matching \"{}\"", needle))?;
    let new_body = format!("{}\n{extra}", entry.body.trim_end());
    crate::datalog::append_entry(&log_path, topic, &new_body, entry.timestamp_min)?;
    crate::datalog::append_delete(&log_path, entry.offset)?;
    Ok(format!("appended to entry matching \"{}\" in {}", needle, topic))
}

/// Append text to entry by 0-based index.
pub fn append_by_index(dir: &Path, topic: &str, idx: usize, extra: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;
    if idx >= entries.len() {
        return Err(format!("index {idx} out of range (topic has {} entries, 0-{})",
            entries.len(), entries.len().saturating_sub(1)));
    }
    let entry = &entries[idx];
    let new_body = format!("{}\n{extra}", entry.body.trim_end());
    crate::datalog::append_entry(&log_path, topic, &new_body, entry.timestamp_min)?;
    crate::datalog::append_delete(&log_path, entry.offset)?;
    Ok(format!("appended to entry [{idx}] in {}", topic))
}

/// Append text to the most recent entry with a given tag.
pub fn append_by_tag(dir: &Path, topic: &str, tag: &str, extra: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;
    let tag_lower = tag.to_lowercase();
    let entry = entries.iter().rev().find(|e| {
        e.body.lines().any(|line| {
            line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']'))
                .map(|inner| inner.split(',').any(|t| t.trim().to_lowercase() == tag_lower))
                .unwrap_or(false)
        })
    }).ok_or_else(|| format!("no entry with tag '{}' in {}", tag, topic))?;
    let new_body = format!("{}\n{extra}", entry.body.trim_end());
    crate::datalog::append_entry(&log_path, topic, &new_body, entry.timestamp_min)?;
    crate::datalog::append_delete(&log_path, entry.offset)?;
    Ok(format!("appended to most recent entry tagged '{}' in {}", tag, topic))
}

/// Rename a topic: rewrite all entries with new name, tombstone old ones.
pub fn rename_topic(dir: &Path, old_name: &str, new_name: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, old_name)?;
    if entries.is_empty() { return Err(format!("topic '{}' not found", old_name)); }
    let all = crate::datalog::iter_live(&log_path)?;
    if all.iter().any(|e| e.topic == new_name) {
        return Err(format!("topic '{}' already has entries", new_name));
    }
    for e in &entries {
        crate::datalog::append_entry(&log_path, new_name, &e.body, e.timestamp_min)?;
        crate::datalog::append_delete(&log_path, e.offset)?;
    }
    Ok(format!("renamed {} → {} ({} entries)", old_name, new_name, entries.len()))
}

/// Add or remove tags on an existing entry.
pub fn tag_entry(
    dir: &Path, topic: &str,
    idx: Option<usize>, needle: Option<&str>,
    add: Option<&str>, remove: Option<&str>,
) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;

    let target_idx = if let Some(i) = idx {
        if i >= entries.len() {
            return Err(format!("index {i} out of range (0-{})", entries.len().saturating_sub(1)));
        }
        i
    } else if let Some(n) = needle {
        let lower = n.to_lowercase();
        entries.iter().position(|e| e.body.to_lowercase().contains(&lower))
            .ok_or_else(|| format!("no entry matching \"{}\"", n))?
    } else {
        return Err("provide index or match_str".into());
    };

    let entry = &entries[target_idx];
    let mut tags: Vec<String> = Vec::new();
    let mut body_lines: Vec<&str> = Vec::new();
    for line in entry.body.lines() {
        if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
            for t in inner.split(',') {
                let t = t.trim().to_lowercase();
                if !t.is_empty() { tags.push(t); }
            }
        } else { body_lines.push(line); }
    }

    if let Some(add_str) = add {
        for t in add_str.split(',') {
            let t = t.trim().to_lowercase();
            if !t.is_empty() && !tags.contains(&t) { tags.push(t); }
        }
    }
    if let Some(rm_str) = remove {
        let rm: Vec<String> = rm_str.split(',').map(|t| t.trim().to_lowercase()).collect();
        tags.retain(|t| !rm.contains(t));
    }
    tags.sort();
    tags.dedup();

    let mut new_body = String::new();
    if !tags.is_empty() { new_body.push_str(&format!("[tags: {}]\n", tags.join(", "))); }
    new_body.push_str(&body_lines.join("\n"));

    crate::datalog::append_entry(&log_path, topic, &new_body, entry.timestamp_min)?;
    crate::datalog::append_delete(&log_path, entry.offset)?;
    Ok(format!("tags updated on entry [{target_idx}] in {}: [{}]", topic, tags.join(", ")))
}

pub fn merge_topics(dir: &Path, from: &str, into: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let from_t = crate::config::sanitize_topic(from);
    let into_t = crate::config::sanitize_topic(into);
    let log_path = crate::config::log_path(dir);
    let entries = crate::datalog::iter_live(&log_path)?;
    let src: Vec<_> = entries.iter().filter(|e| e.topic == from_t).collect();
    if src.is_empty() { return Err(format!("topic '{}' not found", from)); }
    let mut moved = 0;
    for e in &src {
        crate::datalog::append_entry(&log_path, &into_t, &e.body, e.timestamp_min)?;
        crate::datalog::append_delete(&log_path, e.offset)?;
        moved += 1;
    }
    Ok(format!("merged {moved} entries from {from_t} into {into_t}"))
}

fn add_modified_marker(text: &str) -> String {
    let now = crate::time::LocalTime::now();
    format!("[modified: {now}]\n{text}")
}
