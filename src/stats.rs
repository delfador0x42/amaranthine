use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

pub fn list_tags(dir: &Path) -> Result<String, String> {
    crate::cache::with_corpus(dir, |cached| {
        let mut tags: BTreeMap<String, usize> = BTreeMap::new();
        for e in cached {
            if let Some(ref line) = e.tags_raw {
                if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                    for tag in inner.split(',') {
                        let t = tag.trim();
                        if !t.is_empty() { *tags.entry(t.to_string()).or_insert(0) += 1; }
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
        out
    })
}

pub fn stats(dir: &Path) -> Result<String, String> {
    crate::cache::with_corpus(dir, |cached| {
        let mut topics: crate::fxhash::FxHashSet<&str> = crate::fxhash::FxHashSet::default();
        let mut tags: crate::fxhash::FxHashSet<String> = crate::fxhash::FxHashSet::default();
        let mut tagged = 0usize;
        let mut oldest: Option<i32> = None;
        let mut newest: Option<i32> = None;
        for e in cached {
            topics.insert(&e.topic);
            if e.timestamp_min != 0 {
                oldest = Some(oldest.map_or(e.timestamp_min, |o: i32| o.min(e.timestamp_min)));
                newest = Some(newest.map_or(e.timestamp_min, |n: i32| n.max(e.timestamp_min)));
            }
            if let Some(ref line) = e.tags_raw {
                if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                    tagged += 1;
                    for tag in inner.split(',') {
                        let t = tag.trim();
                        if !t.is_empty() { tags.insert(t.to_string()); }
                    }
                }
            }
        }
        let now_days = crate::time::LocalTime::now().to_days();
        let mut out = String::new();
        let _ = writeln!(out, "topics:         {}", topics.len());
        let _ = writeln!(out, "entries:        {}", cached.len());
        let _ = writeln!(out, "tagged entries: {tagged}");
        let _ = writeln!(out, "unique tags:    {}", tags.len());
        if let (Some(o), Some(n)) = (oldest, newest) {
            let _ = writeln!(out, "oldest entry:   {} days ago", now_days - o as i64 / 1440);
            let _ = writeln!(out, "newest entry:   {} days ago", now_days - n as i64 / 1440);
        }
        out
    })
}

pub fn check_stale(dir: &Path) -> Result<String, String> {
    crate::cache::with_corpus(dir, |cached| {
        let mut stale = Vec::new();
        let mut checked = 0usize;
        for e in cached {
            let lines: Vec<&str> = e.body.lines().collect();
            if let Some((ref src_path, _)) = crate::config::parse_source(&lines) {
                checked += 1;
                let date = crate::time::minutes_to_date_str(e.timestamp_min);
                if let Some(msg) = crate::config::check_staleness(src_path, &date) {
                    let preview = lines.iter()
                        .find(|l| !l.starts_with('[') && !l.trim().is_empty())
                        .map(|l| l.trim()).unwrap_or("");
                    let short = if preview.len() > 60 { &preview[..60] } else { preview };
                    stale.push(format!("  [{}] {date}: {msg}\n    {short}", e.topic));
                }
            }
        }
        if stale.is_empty() {
            format!("checked {checked} sourced entries: all fresh")
        } else {
            format!("{} stale of {checked} sourced entries:\n{}", stale.len(), stale.join("\n"))
        }
    })
}

/// For each stale entry, show the full entry text alongside the current source excerpt.
pub fn refresh_stale(dir: &Path) -> Result<String, String> {
    crate::cache::with_corpus(dir, |cached| {
        let mut out = String::new();
        let mut stale_count = 0usize;
        let mut checked = 0usize;
        for e in cached {
            let lines: Vec<&str> = e.body.lines().collect();
            let (src_path, src_line) = match crate::config::parse_source(&lines) {
                Some(pair) => pair,
                None => continue,
            };
            checked += 1;
            let date = crate::time::minutes_to_date_str(e.timestamp_min);
            if crate::config::check_staleness(&src_path, &date).is_none() { continue; }
            stale_count += 1;
            let _ = writeln!(out, "--- STALE [{stale_count}] topic={} (written: {date}) ---", e.topic);
            for line in &lines { let _ = writeln!(out, "  {line}"); }
            let _ = writeln!(out, "--- CURRENT SOURCE: {} ---", src_path);
            let _ = writeln!(out, "{}", source_excerpt(&src_path, src_line, 10));
            let _ = writeln!(out);
        }
        if stale_count == 0 {
            format!("checked {checked} sourced entries: all fresh")
        } else {
            let _ = write!(out, "{stale_count} stale of {checked} sourced entries");
            out
        }
    })
}

fn source_excerpt(path: &str, line: Option<usize>, radius: usize) -> String {
    let resolved = crate::config::resolve_source(path);
    let content = match resolved.and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(c) => c,
        None => return format!("  (file not found: {path})"),
    };
    let file_lines: Vec<&str> = content.lines().collect();
    let center = line.unwrap_or(1).saturating_sub(1).min(file_lines.len().saturating_sub(1));
    let start = center.saturating_sub(radius);
    let end = (center + radius + 1).min(file_lines.len());
    let mut out = String::new();
    for i in start..end {
        let marker = if Some(i + 1) == line { ">" } else { " " };
        let _ = writeln!(out, " {marker}{:>4} {}", i + 1, file_lines[i]);
    }
    out
}

pub fn get_entry(dir: &Path, topic: &str, idx: usize) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;
    if entries.is_empty() { return Err(format!("topic '{}' not found", topic)); }
    if idx >= entries.len() {
        return Err(format!("index {idx} out of range (topic has {} entries, 0-{})",
            entries.len(), entries.len().saturating_sub(1)));
    }
    let e = &entries[idx];
    let date = crate::time::minutes_to_date_str(e.timestamp_min);
    Ok(format!("## {date}\n{}", e.body))
}

pub fn list_entries(dir: &Path, topic: &str, match_str: Option<&str>) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;
    if entries.is_empty() { return Err(format!("topic '{}' not found", topic)); }
    let mut out = String::new();
    let mut shown = 0;
    for (i, e) in entries.iter().enumerate() {
        if let Some(needle) = match_str {
            if !e.body.to_lowercase().contains(&needle.to_lowercase()) { continue; }
        }
        shown += 1;
        let date = crate::time::minutes_to_date_str(e.timestamp_min);
        let preview = e.body.lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with("[tags:"))
            .map(|l| {
                let t = l.trim().trim_start_matches("- ");
                if t.len() > 70 { &t[..70] } else { t }
            })
            .unwrap_or("(empty)");
        let _ = writeln!(out, "  [{i}] ## {date} — {preview}");
    }
    if shown == 0 {
        let _ = writeln!(out, "no entries{}", match_str.map(|s| format!(" matching \"{s}\"")).unwrap_or_default());
    } else {
        let _ = writeln!(out, "\n{shown} of {} entries shown", entries.len());
    }
    Ok(out)
}
