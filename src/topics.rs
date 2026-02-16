use crate::time;
use std::fmt::Write;
use std::fs;
use std::path::Path;

pub fn list(dir: &Path) -> Result<String, String> {
    list_inner(dir, false)
}

pub fn list_compact(dir: &Path) -> Result<String, String> {
    list_inner(dir, true)
}

fn list_inner(dir: &Path, compact: bool) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let files = crate::config::list_topic_files(dir)?;
    let mut out = String::new();
    if files.is_empty() {
        let _ = writeln!(out, "no topic files in {}", dir.display());
        return Ok(out);
    }

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();
        let entries = content.lines().filter(|l| l.starts_with("## ")).count();
        let tags = collect_tags(&content);
        let tag_str = if tags.is_empty() { String::new() } else { format!(" [tags: {}]", tags.join(", ")) };
        if compact {
            let _ = writeln!(out, "  {name:<24} {entries:>3} entries{tag_str}");
        } else {
            let preview = last_entry_preview(&content);
            let _ = writeln!(out, "  {name:<24} {entries:>3} entries  |{tag_str} {preview}");
        }
    }
    Ok(out)
}

/// Collect unique tags from all [tags: ...] lines in a file.
fn collect_tags(content: &str) -> Vec<String> {
    let mut tags = std::collections::BTreeSet::new();
    for line in content.lines() {
        if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
            for tag in inner.split(',') {
                let t = tag.trim().to_lowercase();
                if !t.is_empty() { tags.insert(t); }
            }
        }
    }
    tags.into_iter().collect()
}

fn last_entry_preview(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let last_header = lines.iter().rposition(|l| l.starts_with("## "));
    if let Some(idx) = last_header {
        for line in &lines[idx + 1..] {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with("[tags:") {
                let clean = trimmed.trim_start_matches("- ");
                if clean.len() > 60 {
                    let mut end = 60;
                    while end > 0 && !clean.is_char_boundary(end) { end -= 1; }
                    return format!("{}...", &clean[..end]);
                }
                return clean.to_string();
            }
        }
    }
    "(empty)".to_string()
}

pub fn recent(dir: &Path, days: u64, plain: bool) -> Result<String, String> {
    recent_inner(dir, Some(days), None, plain)
}

pub fn recent_hours(dir: &Path, hours: u64, plain: bool) -> Result<String, String> {
    recent_inner(dir, None, Some(hours), plain)
}

fn recent_inner(dir: &Path, days: Option<u64>, hours: Option<u64>, plain: bool) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let now = time::LocalTime::now();
    // Use minutes-based comparison for hours, days-based for days
    let use_minutes = hours.is_some();
    let cutoff_min = now.to_minutes() - hours.unwrap_or(0) as i64 * 60;
    let cutoff_day = now.to_days() - days.unwrap_or(7) as i64;

    let files = crate::config::list_topic_files(dir)?;
    let mut found = 0;
    let mut out = String::new();

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();
        let mut in_recent = false;

        for line in content.lines() {
            if line.starts_with("## ") {
                let header = line.trim_start_matches("## ");
                in_recent = if use_minutes {
                    time::parse_date_minutes(header)
                        .map(|m| m >= cutoff_min)
                        .unwrap_or(false)
                } else {
                    time::parse_date_days(header)
                        .map(|d| d >= cutoff_day)
                        .unwrap_or(false)
                };
                if in_recent {
                    if plain {
                        let _ = writeln!(out, "[{name}] {line}");
                    } else {
                        let _ = writeln!(out, "\x1b[1;36m[{name}]\x1b[0m {line}");
                    }
                    found += 1;
                }
            } else if in_recent && !line.is_empty() {
                let _ = writeln!(out, "  {line}");
            }
        }
    }

    if found == 0 {
        let label = if use_minutes {
            format!("{} hours", hours.unwrap_or(0))
        } else {
            format!("{} days", days.unwrap_or(7))
        };
        let _ = writeln!(out, "no entries in the last {label}");
    }
    Ok(out)
}
