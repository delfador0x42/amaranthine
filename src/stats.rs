use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::Path;

/// List all tags used across all topics with counts.
pub fn list_tags(dir: &Path) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }
    let files = crate::config::list_topic_files(dir)?;
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        for line in content.lines() {
            if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                for tag in inner.split(',') {
                    let t = tag.trim().to_lowercase();
                    if !t.is_empty() {
                        *tags.entry(t).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let mut out = String::new();
    if tags.is_empty() {
        let _ = writeln!(out, "no tags found");
    } else {
        for (tag, count) in &tags {
            let _ = writeln!(out, "  {tag:<24} {count} entries");
        }
        let _ = writeln!(out, "\n{} unique tags across {} entries", tags.len(), tags.values().sum::<usize>());
    }
    Ok(out)
}

/// Show stats: topic count, entry count, date range, tag count.
pub fn stats(dir: &Path) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }
    let files = crate::config::list_topic_files(dir)?;
    let mut total_entries = 0;
    let mut total_lines = 0;
    let mut oldest: Option<i64> = None;
    let mut newest: Option<i64> = None;
    let mut tags: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut tagged_entries = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        total_lines += content.lines().count();
        for line in content.lines() {
            if line.starts_with("## ") {
                total_entries += 1;
                if let Some(d) = crate::time::parse_date_days(line.trim_start_matches("## ")) {
                    oldest = Some(oldest.map_or(d, |o: i64| o.min(d)));
                    newest = Some(newest.map_or(d, |n: i64| n.max(d)));
                }
            }
            if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                tagged_entries += 1;
                for tag in inner.split(',') {
                    let t = tag.trim().to_lowercase();
                    if !t.is_empty() { tags.insert(t); }
                }
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "topics:         {}", files.len());
    let _ = writeln!(out, "entries:        {total_entries}");
    let _ = writeln!(out, "total lines:    {total_lines}");
    let _ = writeln!(out, "tagged entries: {tagged_entries}");
    let _ = writeln!(out, "unique tags:    {}", tags.len());
    if let (Some(o), Some(n)) = (oldest, newest) {
        let _ = writeln!(out, "oldest entry:   {} days ago", crate::time::LocalTime::now().to_days() - o);
        let _ = writeln!(out, "newest entry:   {} days ago", crate::time::LocalTime::now().to_days() - n);
    }
    Ok(out)
}

/// List entries in a topic, optionally filtered by match_str. For bulk review.
pub fn list_entries(dir: &Path, topic: &str, match_str: Option<&str>) -> Result<String, String> {
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));
    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = crate::delete::split_sections(&content);
    let mut out = String::new();
    let mut shown = 0;

    for (i, (hdr, body)) in sections.iter().enumerate() {
        let entry_text = format!("{hdr}\n{body}");
        if let Some(needle) = match_str {
            if !entry_text.to_lowercase().contains(&needle.to_lowercase()) {
                continue;
            }
        }
        shown += 1;
        let preview = body.lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with("[tags:"))
            .map(|l| {
                let t = l.trim().trim_start_matches("- ");
                if t.len() > 70 { &t[..70] } else { t }
            })
            .unwrap_or("(empty)");
        let _ = writeln!(out, "  [{i}] {hdr} — {preview}");
    }

    if shown == 0 {
        let _ = writeln!(out, "no entries{}", match_str.map(|s| format!(" matching \"{s}\"")).unwrap_or_default());
    } else {
        let _ = writeln!(out, "\n{shown} of {} entries shown", sections.len());
    }
    Ok(out)
}
