use std::path::Path;

/// Delete entries from a topic via data.log tombstones.
pub fn run(dir: &Path, topic: &str, last: bool, all: bool, match_str: Option<&str>) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = topic_entries(&log_path, topic)?;

    if entries.is_empty() { return Err(format!("topic '{}' not found", topic)); }

    if all {
        for e in &entries { crate::datalog::append_delete(&log_path, e.offset)?; }
        return Ok(format!("deleted {} ({} entries removed)", topic, entries.len()));
    }

    if let Some(needle) = match_str {
        let lower = needle.to_lowercase();
        let entry = entries.iter().find(|e| e.body.to_lowercase().contains(&lower))
            .ok_or_else(|| format!("no entry matching \"{}\"", needle))?;
        crate::datalog::append_delete(&log_path, entry.offset)?;
        return Ok(format!("removed entry matching \"{}\" from {} ({} remaining)",
            needle, topic, entries.len() - 1));
    }

    if !last { return Err("specify --last, --all, or --match <substring>".into()); }

    let last_entry = entries.last().unwrap();
    crate::datalog::append_delete(&log_path, last_entry.offset)?;
    Ok(format!("removed last entry from {} ({} remaining)", topic, entries.len() - 1))
}

/// Delete entry by 0-based index.
pub fn run_by_index(dir: &Path, topic: &str, idx: usize) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = topic_entries(&log_path, topic)?;

    if idx >= entries.len() {
        return Err(format!("index {idx} out of range (topic has {} entries, 0-{})",
            entries.len(), entries.len().saturating_sub(1)));
    }

    crate::datalog::append_delete(&log_path, entries[idx].offset)?;
    Ok(format!("removed entry [{idx}] from {} ({} remaining)", topic, entries.len() - 1))
}

/// Get all live entries for a topic from data.log, in log order.
pub fn topic_entries(log_path: &Path, topic: &str) -> Result<Vec<crate::datalog::LogEntry>, String> {
    let all = crate::datalog::iter_live(log_path)?;
    Ok(all.into_iter().filter(|e| e.topic == topic).collect())
}

// --- Legacy: split_sections for .md migration compatibility ---

fn is_header_at(bytes: &[u8], pos: usize) -> bool {
    pos + 7 < bytes.len()
        && bytes[pos] == b'#' && bytes[pos + 1] == b'#' && bytes[pos + 2] == b' '
        && bytes[pos + 3].is_ascii_digit() && bytes[pos + 4].is_ascii_digit()
        && bytes[pos + 5].is_ascii_digit() && bytes[pos + 6].is_ascii_digit()
        && bytes[pos + 7] == b'-'
}

fn find_next_header(content: &str, from: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut pos = from;
    while pos < bytes.len() {
        match content[pos..].find("\n## ") {
            Some(p) => {
                let abs = pos + p + 1;
                if is_header_at(bytes, abs) { return Some(abs); }
                pos = abs + 3;
            }
            None => break,
        }
    }
    None
}

/// Split .md content into (header_line, body) pairs. Used by migration.
pub fn split_sections(content: &str) -> Vec<(&str, &str)> {
    let mut sections = Vec::new();
    let mut i = 0;
    let bytes = content.as_bytes();
    while i < bytes.len() {
        let hdr_start = if is_header_at(bytes, i) { Some(i) }
            else { find_next_header(content, i) };
        let hdr_start = match hdr_start { Some(s) => s, None => break };
        let hdr_end = content[hdr_start..].find('\n')
            .map(|p| hdr_start + p).unwrap_or(content.len());
        let header = &content[hdr_start..hdr_end];
        let body_end = find_next_header(content, hdr_end).unwrap_or(content.len());
        sections.push((header, &content[hdr_end..body_end]));
        i = body_end;
    }
    sections
}
