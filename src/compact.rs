use std::fmt::Write;
use std::path::Path;

/// Find duplicate/similar entries within a topic and optionally merge them.
pub fn run(dir: &Path, topic: &str, apply: bool) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::delete::topic_entries(&log_path, topic)?;
    if entries.is_empty() { return Err(format!("topic '{}' not found", topic)); }
    if entries.len() < 2 {
        return Ok(format!("{topic}: {} entry, nothing to compact", entries.len()));
    }

    let mut pairs: Vec<(usize, usize, f64)> = Vec::new();
    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            let sim = similarity(&entries[i].body, &entries[j].body);
            if sim > 0.5 { pairs.push((i, j, sim)); }
        }
    }
    if pairs.is_empty() {
        return Ok(format!("{topic}: {} entries, no duplicates found", entries.len()));
    }

    let mut out = String::new();
    let _ = writeln!(out, "{topic}: {} similar pair(s) found", pairs.len());
    for (i, j, sim) in &pairs {
        let _ = writeln!(out, "  [{i}] {}", entry_preview(&entries[*i].body));
        let _ = writeln!(out, "  [{j}] {}", entry_preview(&entries[*j].body));
        let _ = writeln!(out, "  similarity: {:.0}%\n", sim * 100.0);
    }
    if !apply {
        let _ = writeln!(out, "run with apply=true to merge (keeps newer, combines bodies)");
        return Ok(out);
    }

    let mut skip: Vec<usize> = Vec::new();
    for (i, j, _) in &pairs {
        if skip.contains(i) || skip.contains(j) { continue; }
        let combined = merge_bodies(&entries[*i].body, &entries[*j].body);
        crate::datalog::append_entry(&log_path, topic, &combined, entries[*j].timestamp_min)?;
        crate::datalog::append_delete(&log_path, entries[*i].offset)?;
        crate::datalog::append_delete(&log_path, entries[*j].offset)?;
        skip.push(*i);
    }
    let _ = writeln!(out, "compacted: merged {} pairs", skip.len());
    Ok(out)
}

/// Scan all topics for compaction opportunities.
pub fn scan(dir: &Path) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    let entries = crate::datalog::iter_live(&log_path)?;
    let mut topics: std::collections::BTreeMap<String, Vec<&crate::datalog::LogEntry>> =
        std::collections::BTreeMap::new();
    for e in &entries { topics.entry(e.topic.clone()).or_default().push(e); }

    let mut out = String::new();
    let mut total_dupes = 0;
    for (name, group) in &topics {
        let mut dupes = 0;
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                if similarity(&group[i].body, &group[j].body) > 0.5 { dupes += 1; }
            }
        }
        if dupes > 0 {
            let _ = writeln!(out, "  {name}: {dupes} similar pair(s) in {} entries", group.len());
            total_dupes += dupes;
        }
    }
    if total_dupes == 0 {
        let _ = writeln!(out, "no duplicates found across {} topics", topics.len());
    } else {
        let _ = writeln!(out, "\n{total_dupes} total similar pair(s) — use compact <topic> to review");
    }
    Ok(out)
}

fn similarity(a: &str, b: &str) -> f64 {
    let al = a.to_lowercase();
    let bl = b.to_lowercase();
    let sa: std::collections::HashSet<&str> = al.split_whitespace().filter(|w| w.len() >= 4).collect();
    let sb: std::collections::HashSet<&str> = bl.split_whitespace().filter(|w| w.len() >= 4).collect();
    if sa.is_empty() || sb.is_empty() { return 0.0; }
    sa.intersection(&sb).count() as f64 / sa.len().min(sb.len()) as f64
}

fn entry_preview(body: &str) -> String {
    body.lines()
        .find(|l| !l.trim().is_empty() && !crate::text::is_metadata_line(l))
        .map(|l| {
            let t = l.trim().trim_start_matches("- ");
            if t.len() > 60 { format!("{}...", &t[..60]) } else { t.to_string() }
        })
        .unwrap_or_else(|| "(empty)".into())
}

fn merge_bodies(older: &str, newer: &str) -> String {
    let newer_lines: Vec<&str> = newer.trim().lines().collect();
    let mut result = newer.trim().to_string();
    for line in older.trim().lines() {
        if crate::text::is_metadata_line(line) { continue; }
        if !newer_lines.iter().any(|n| n.trim() == line.trim()) && !line.trim().is_empty() {
            result.push('\n');
            result.push_str(line);
        }
    }
    result
}
