//! v5 Compression engine: dedup across topics, temporal chain detection,
//! source pointer extraction. Turns raw entries into dense compressed facts.

use std::collections::BTreeMap;

/// Input: one matching entry collected by the orchestrator.
pub struct RawEntry {
    pub topic: String,
    pub body: String,
    pub timestamp_min: i32,
    pub days_old: i64,
    pub tags: Vec<String>,
    pub relevance: f64,
}

/// Output: a compressed fact ready for the briefing formatter.
pub struct Compressed {
    pub topic: String,
    pub body: String,
    pub date: String,
    pub days_old: i64,
    pub tags: Vec<String>,
    pub relevance: f64,
    pub source: Option<String>,
    pub chain: Option<String>,
    pub also_in: Vec<String>,
}

/// Run all compression passes. Returns compressed entries sorted by relevance.
pub fn compress(entries: Vec<RawEntry>) -> Vec<Compressed> {
    let mut out: Vec<Compressed> = entries.into_iter().map(|e| {
        let source = crate::text::extract_source(&e.body);
        let date = crate::time::minutes_to_date_str(e.timestamp_min);
        Compressed {
            topic: e.topic, body: e.body, date, days_old: e.days_old,
            tags: e.tags, relevance: e.relevance, source,
            chain: None, also_in: Vec::new(),
        }
    }).collect();
    dedup(&mut out);
    temporal_chains(&mut out);
    out.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// First non-metadata content line of an entry body.
pub fn first_content(body: &str) -> &str {
    body.lines().find(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with("[tags:") && !t.starts_with("[source:")
            && !t.starts_with("[type:") && !t.starts_with("[modified:")
            && !t.starts_with("[tier:")
    }).unwrap_or("")
}

/// Cross-topic dedup: identical first content lines → merge with provenance.
fn dedup(entries: &mut Vec<Compressed>) {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        let key = first_content(&e.body).to_lowercase();
        if key.len() >= 10 { groups.entry(key).or_default().push(i); }
    }
    let mut remove = Vec::new();
    for indices in groups.values() {
        if indices.len() < 2 { continue; }
        let topics: Vec<&str> = indices.iter().map(|&i| entries[i].topic.as_str()).collect();
        if topics.windows(2).all(|w| w[0] == w[1]) { continue; }
        let best = *indices.iter().max_by(|a, b|
            entries[**a].relevance.partial_cmp(&entries[**b].relevance)
                .unwrap_or(std::cmp::Ordering::Equal)).unwrap();
        let others: Vec<(usize, String)> = indices.iter()
            .filter(|&&i| i != best && entries[i].topic != entries[best].topic)
            .map(|&i| (i, entries[i].topic.clone())).collect();
        for (idx, topic) in others {
            entries[best].also_in.push(topic);
            remove.push(idx);
        }
    }
    remove.sort_unstable();
    remove.dedup();
    for &idx in remove.iter().rev() { entries.remove(idx); }
}

/// Temporal chains: same topic + same dominant entity → compress to timeline.
fn temporal_chains(entries: &mut Vec<Compressed>) {
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(term) = dominant_term(first_content(&e.body)) {
            groups.entry((e.topic.clone(), term)).or_default().push(i);
        }
    }
    let mut remove = Vec::new();
    for ((_, term), indices) in &groups {
        if indices.len() < 2 { continue; }
        // Sort oldest first (highest days_old = oldest)
        let mut sorted: Vec<usize> = indices.clone();
        sorted.sort_by(|a, b| entries[*b].days_old.cmp(&entries[*a].days_old));
        let steps: Vec<String> = sorted.iter().map(|&i| {
            let fc = first_content(&entries[i].body);
            let without = fc.replace(term.as_str(), "");
            let words: Vec<&str> = without.split_whitespace().take(5).collect();
            let step = words.join(" ");
            if step.is_empty() { entries[i].date[5..].to_string() }
            else { format!("{} ({})", step, &entries[i].date[5..]) }
        }).collect();
        let chain = format!("{}: {}", term, steps.join(" → "));
        let newest = *sorted.last().unwrap();
        entries[newest].chain = Some(chain);
        entries[newest].relevance += sorted.len() as f64;
        for &idx in sorted.iter().take(sorted.len() - 1) { remove.push(idx); }
    }
    remove.sort_unstable();
    remove.dedup();
    for &idx in remove.iter().rev() { entries.remove(idx); }
}

/// Longest capitalized or all-caps word — the likely entity name.
fn dominant_term(line: &str) -> Option<String> {
    line.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() >= 3)
        .filter(|w| w.chars().next().map_or(false, |c| c.is_uppercase()))
        .max_by_key(|w| w.len())
        .map(|w| w.to_string())
}
