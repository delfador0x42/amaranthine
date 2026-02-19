use std::fmt::Write;
use std::path::Path;

pub fn run(dir: &Path) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Ok("no data.log found\n".into()); }
    let entries = crate::datalog::iter_live(&log_path)?;
    if entries.is_empty() { return Ok("no entries\n".into()); }

    // Group by topic preserving order
    let mut topic_order: Vec<String> = Vec::new();
    let mut grouped: std::collections::BTreeMap<String, Vec<&crate::datalog::LogEntry>> =
        std::collections::BTreeMap::new();
    for e in &entries {
        if !grouped.contains_key(&e.topic) { topic_order.push(e.topic.clone()); }
        grouped.entry(e.topic.clone()).or_default().push(e);
    }

    let mut out = String::new();
    for (i, name) in topic_order.iter().enumerate() {
        let group = &grouped[name];
        if i > 0 { let _ = writeln!(out); }
        let latest = group.last().map(|e| crate::time::minutes_to_date_str(e.timestamp_min))
            .unwrap_or_else(|| "empty".into());
        let _ = writeln!(out, "### {} ({} entries, last: {})", name, group.len(), latest);
        for e in group {
            let preview = e.body.lines()
                .find(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with("[tags:") && !t.starts_with("[source:")
                        && !t.starts_with("[type:") && !t.starts_with("[modified:")
                })
                .unwrap_or("(empty)");
            let _ = writeln!(out, "- {}", truncate(preview.trim().trim_start_matches("- "), 100));
        }
    }
    Ok(out)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}
