//! v5 Reconstruct: one-shot compressed briefing.
//! Single scan of data.log → compress → format LLM-native output.

use std::collections::BTreeSet;
use std::path::Path;
use crate::compress::RawEntry;

pub fn run(dir: &Path, query: &str) -> Result<String, String> {
    let q = crate::config::sanitize_topic(query);
    let q_terms = crate::text::query_terms(query);
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Err("no data.log found".into()); }
    let all = crate::datalog::iter_live(&log_path)?;
    let now_days = crate::time::LocalTime::now().to_days();

    // Identify primary topics (name contains query)
    let mut topic_set = BTreeSet::new();
    for e in &all { topic_set.insert(e.topic.as_str()); }
    let primary: Vec<String> = topic_set.iter()
        .filter(|n| n.contains(q.as_str()))
        .map(|s| s.to_string()).collect();

    // Collect matching entries in one scan
    let mut entries: Vec<RawEntry> = Vec::new();
    for e in &all {
        let is_primary = primary.iter().any(|p| p == &e.topic);
        let lower = e.body.to_lowercase();
        let is_related = !q_terms.is_empty()
            && q_terms.iter().any(|t| lower.contains(t.as_str()));
        if !is_primary && !is_related { continue; }
        let days_old = now_days - e.timestamp_min as i64 / 1440;
        let mut relevance = if is_primary { 10.0 } else { 0.0 };
        for t in &q_terms { relevance += lower.matches(t.as_str()).count() as f64; }
        entries.push(RawEntry {
            topic: e.topic.clone(), body: e.body.clone(),
            timestamp_min: e.timestamp_min, days_old,
            tags: extract_tags(&e.body), relevance,
        });
    }
    if entries.is_empty() { return Ok(format!("No entries found for '{query}'.\n")); }

    let raw_count = entries.len();
    let compressed = crate::compress::compress(entries);
    Ok(crate::briefing::format(&compressed, query, raw_count, &primary))
}

fn extract_tags(body: &str) -> Vec<String> {
    body.lines().filter_map(|l| l.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')))
        .flat_map(|inner| inner.split(',').map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty()).collect::<Vec<_>>())
        .collect()
}
