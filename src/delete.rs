use std::fs;
use std::path::Path;

pub fn run(dir: &Path, topic: &str, last: bool, all: bool, match_str: Option<&str>) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    if all {
        fs::remove_file(&filepath).map_err(|e| e.to_string())?;
        return Ok(format!("deleted {filename}.md"));
    }

    if let Some(needle) = match_str {
        return delete_matching(&filepath, &filename, needle);
    }

    if !last {
        return Err("specify --last, --all, or --match <substring>".into());
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    match content.rfind("\n## ") {
        Some(pos) => {
            let trimmed = content[..pos].trim_end();
            crate::config::atomic_write(&filepath, &format!("{trimmed}\n"))?;
            let remaining = trimmed.matches("\n## ").count();
            Ok(format!("removed last entry from {filename}.md ({remaining} remaining)"))
        }
        None => Err("no entries to remove".into()),
    }
}

/// Delete entry by 0-based index (from list_entries).
pub fn run_by_index(dir: &Path, topic: &str, idx: usize) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = split_sections(&content);

    if idx >= sections.len() {
        return Err(format!("index {idx} out of range (topic has {} entries, 0-{})",
            sections.len(), sections.len().saturating_sub(1)));
    }

    let result = rebuild_file(&content, &sections, Some(idx), None);
    crate::config::atomic_write(&filepath, &result)?;

    let remaining = result.matches("\n## ").count();
    Ok(format!("removed entry [{idx}] from {filename}.md ({remaining} remaining)"))
}

fn delete_matching(filepath: &Path, filename: &str, needle: &str) -> Result<String, String> {
    let content = fs::read_to_string(filepath).map_err(|e| e.to_string())?;
    let sections = split_sections(&content);
    let lower = needle.to_lowercase();

    let idx = sections.iter().position(|(_, body)| body.to_lowercase().contains(&lower));
    let idx = match idx {
        Some(i) => i,
        None => return Err(format!("no entry matching \"{needle}\"")),
    };

    let result = rebuild_file(&content, &sections, Some(idx), None);
    crate::config::atomic_write(filepath, &result)?;

    let remaining = result.matches("\n## ").count();
    Ok(format!("removed entry matching \"{needle}\" from {filename}.md ({remaining} remaining)"))
}

/// Rebuild a topic file from sections.
/// `skip` = index to omit, `replace` = (index, new_body) to swap content.
pub fn rebuild_file(
    content: &str,
    sections: &[(&str, &str)],
    skip: Option<usize>,
    replace: Option<(usize, &str)>,
) -> String {
    // Extract # title header
    let title = content.lines().next().filter(|l| l.starts_with("# ")).unwrap_or("");
    let mut result = format!("{title}\n");

    for (i, (hdr, body)) in sections.iter().enumerate() {
        if skip == Some(i) { continue; }
        result.push('\n');
        result.push_str(hdr);
        result.push('\n');
        if let Some((ri, new_body)) = replace {
            if ri == i {
                result.push_str(new_body);
                result.push('\n');
                continue;
            }
        }
        // Original body: strip leading \n, keep content
        let trimmed = body.strip_prefix('\n').unwrap_or(body);
        result.push_str(trimmed);
        if !result.ends_with('\n') { result.push('\n'); }
    }
    result
}

/// Check if position in bytes starts an entry header "## YYYY-".
fn is_header_at(bytes: &[u8], pos: usize) -> bool {
    pos + 7 < bytes.len()
        && bytes[pos] == b'#' && bytes[pos + 1] == b'#' && bytes[pos + 2] == b' '
        && bytes[pos + 3].is_ascii_digit() && bytes[pos + 4].is_ascii_digit()
        && bytes[pos + 5].is_ascii_digit() && bytes[pos + 6].is_ascii_digit()
        && bytes[pos + 7] == b'-'
}

/// Find next entry header ("\n## YYYY-") starting from `from`. Returns position of "##".
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

/// Split content into (header_line, body) pairs for each `## YYYY-MM-DD` section.
/// Only splits on proper entry headers, not "## " in body text.
pub fn split_sections(content: &str) -> Vec<(&str, &str)> {
    let mut sections = Vec::new();
    let mut i = 0;
    let bytes = content.as_bytes();

    while i < bytes.len() {
        let hdr_start = if is_header_at(bytes, i) {
            Some(i)
        } else {
            find_next_header(content, i)
        };

        let hdr_start = match hdr_start {
            Some(s) => s,
            None => break,
        };

        let hdr_end = content[hdr_start..].find('\n')
            .map(|p| hdr_start + p)
            .unwrap_or(content.len());

        let header = &content[hdr_start..hdr_end];
        let body_end = find_next_header(content, hdr_end)
            .unwrap_or(content.len());

        let body = &content[hdr_end..body_end];
        sections.push((header, body));
        i = body_end;
    }
    sections
}
