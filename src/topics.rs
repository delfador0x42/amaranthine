use crate::time;
use std::fmt::Write;
use std::path::Path;

pub fn list(dir: &Path) -> Result<String, String> {
    list_inner(dir, false)
}

pub fn list_compact(dir: &Path) -> Result<String, String> {
    list_inner(dir, true)
}

fn list_inner(dir: &Path, compact: bool) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Ok("no data.log found\n".into()); }
    let entries = crate::datalog::iter_live(&log_path)?;
    if entries.is_empty() { return Ok("no entries\n".into()); }

    let mut topics: std::collections::BTreeMap<String, TopicInfo> = std::collections::BTreeMap::new();
    for e in &entries {
        let info = topics.entry(e.topic.clone()).or_default();
        info.count += 1;
        collect_tags_from_body(&e.body, &mut info.tags);
        info.last_preview = entry_preview(&e.body);
    }

    let mut out = String::new();
    for (name, info) in &topics {
        let tag_str = if info.tags.is_empty() { String::new() }
            else { format!(" [tags: {}]", info.tags.iter().cloned().collect::<Vec<_>>().join(", ")) };
        if compact {
            let _ = writeln!(out, "  {name:<24} {:>3} entries{tag_str}", info.count);
        } else {
            let _ = writeln!(out, "  {name:<24} {:>3} entries  |{tag_str} {}", info.count, info.last_preview);
        }
    }
    Ok(out)
}

#[derive(Default)]
struct TopicInfo {
    count: usize,
    tags: std::collections::BTreeSet<String>,
    last_preview: String,
}

fn collect_tags_from_body(body: &str, tags: &mut std::collections::BTreeSet<String>) {
    for line in body.lines() {
        if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
            for tag in inner.split(',') {
                let t = tag.trim().to_lowercase();
                if !t.is_empty() { tags.insert(t); }
            }
        }
    }
}

fn entry_preview(body: &str) -> String {
    body.lines()
        .find(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("[tags:") && !t.starts_with("[source:")
                && !t.starts_with("[type:") && !t.starts_with("[modified:")
        })
        .map(|l| {
            let clean = l.trim().trim_start_matches("- ");
            if clean.len() > 60 {
                let mut end = 60;
                while end > 0 && !clean.is_char_boundary(end) { end -= 1; }
                format!("{}...", &clean[..end])
            } else { clean.to_string() }
        })
        .unwrap_or_else(|| "(empty)".into())
}

pub fn read_topic(dir: &Path, topic: &str) -> Result<String, String> {
    let f = crate::config::sanitize_topic(topic);
    let log_path = crate::config::log_path(dir);
    let entries = crate::datalog::iter_live(&log_path)?;
    let group: Vec<_> = entries.iter().filter(|e| e.topic == f).collect();
    if group.is_empty() { return Err(format!("topic '{f}' not found")); }
    let mut out = String::new();
    for e in &group {
        let date = time::minutes_to_date_str(e.timestamp_min);
        out.push_str(&format!("## {date}\n{}\n\n", e.body.trim()));
    }
    Ok(out)
}

pub fn recent(dir: &Path, days: u64, plain: bool) -> Result<String, String> {
    recent_inner(dir, Some(days), None, plain)
}

pub fn recent_hours(dir: &Path, hours: u64, plain: bool) -> Result<String, String> {
    recent_inner(dir, None, Some(hours), plain)
}

fn recent_inner(dir: &Path, days: Option<u64>, hours: Option<u64>, plain: bool) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Ok("no data.log found\n".into()); }
    let entries = crate::datalog::iter_live(&log_path)?;
    let now = time::LocalTime::now();
    let use_minutes = hours.is_some();
    let cutoff_min = now.to_minutes() - hours.unwrap_or(0) as i64 * 60;
    let cutoff_day = now.to_days() - days.unwrap_or(7) as i64;

    let mut found = 0;
    let mut out = String::new();
    for e in &entries {
        let is_recent = if use_minutes {
            e.timestamp_min as i64 >= cutoff_min
        } else {
            e.timestamp_min as i64 / 1440 >= cutoff_day
        };
        if !is_recent { continue; }
        let date = time::minutes_to_date_str(e.timestamp_min);
        if plain {
            let _ = writeln!(out, "[{}] ## {}", e.topic, date);
        } else {
            let _ = writeln!(out, "\x1b[1;36m[{}]\x1b[0m ## {}", e.topic, date);
        }
        for line in e.body.lines() {
            if !line.is_empty() { let _ = writeln!(out, "  {line}"); }
        }
        found += 1;
    }
    if found == 0 {
        let label = if use_minutes { format!("{} hours", hours.unwrap_or(0)) }
            else { format!("{} days", days.unwrap_or(7)) };
        let _ = writeln!(out, "no entries in the last {label}");
    }
    Ok(out)
}
