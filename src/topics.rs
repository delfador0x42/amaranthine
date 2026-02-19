use crate::time;
use std::fmt::Write;
use std::path::Path;

pub fn list(dir: &Path) -> Result<String, String> {
    list_inner(dir, false)
}

pub fn list_compact(dir: &Path) -> Result<String, String> {
    // Fast path: read topic table from binary index (zero corpus scan)
    let from_index = crate::mcp::with_index(|data| {
        crate::binquery::topic_table(data).ok()
    }).flatten().or_else(|| {
        std::fs::read(dir.join("index.bin")).ok()
            .and_then(|data| crate::binquery::topic_table(&data).ok())
    });
    if let Some(topics) = from_index {
        let mut out = String::new();
        for (_, name, count) in &topics {
            let _ = writeln!(out, "  {:<24} {:>3} entries", name, count);
        }
        return Ok(out);
    }
    // Fallback: corpus scan
    list_inner(dir, true)
}

fn list_inner(dir: &Path, compact: bool) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Ok("no data.log found\n".into()); }
    crate::cache::with_corpus(dir, |cached| {
        if cached.is_empty() { return "no entries\n".into(); }
        let mut topics: std::collections::BTreeMap<String, TopicInfo> = std::collections::BTreeMap::new();
        for e in cached {
            let info = topics.entry(e.topic.to_string()).or_default();
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
        out
    })
}

#[derive(Default)]
struct TopicInfo {
    count: usize,
    tags: std::collections::BTreeSet<String>,
    last_preview: String,
}

fn collect_tags_from_body(body: &str, tags: &mut std::collections::BTreeSet<String>) {
    for line in body.lines() {
        for t in crate::text::parse_tags_raw(Some(line)) {
            tags.insert(t.to_string());
        }
    }
}

fn entry_preview(body: &str) -> String {
    body.lines()
        .find(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("[tags:") && !t.starts_with("[source:")
                && !t.starts_with("[type:") && !t.starts_with("[modified:")
                && !t.starts_with("[confidence:") && !t.starts_with("[links:")
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
    crate::cache::with_corpus(dir, |cached| {
        let group: Vec<_> = cached.iter().filter(|e| e.topic == f).collect();
        if group.is_empty() { return Err(format!("topic '{f}' not found")); }
        let mut out = String::new();
        for e in &group {
            out.push_str(&format!("## {}\n{}\n\n", e.date_str(), e.body.trim()));
        }
        Ok(out)
    })?
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
    crate::cache::with_corpus(dir, |cached| {
        let now = time::LocalTime::now();
        let use_minutes = hours.is_some();
        let cutoff_min = now.to_minutes() - hours.unwrap_or(0) as i64 * 60;
        let cutoff_day = now.to_days() - days.unwrap_or(7) as i64;

        let mut found = 0;
        let mut out = String::new();
        for e in cached {
            let is_recent = if use_minutes {
                e.timestamp_min as i64 >= cutoff_min
            } else {
                e.day() >= cutoff_day
            };
            if !is_recent { continue; }
            let date = e.date_str();
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
        out
    })
}
