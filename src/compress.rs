//! v5 Compression engine: dedup across topics, temporal chain detection,
//! source pointer extraction. Turns raw entries into dense compressed facts.

use std::collections::BTreeMap;
use crate::fxhash::FxHashSet;

/// Input: one matching entry collected by the orchestrator.
pub struct RawEntry {
    pub topic: String,
    pub body: String,
    pub timestamp_min: i32,
    pub days_old: i64,
    pub tags: Vec<String>,
    pub relevance: f64,
    pub confidence: f64,
    pub link_in: u16,
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
    pub confidence: f64,
    pub link_in: u16,
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
            confidence: e.confidence, link_in: e.link_in,
        }
    }).collect();
    dedup(&mut out);
    // Tokenize first_content once, reuse across supersede + temporal_chains
    let tokens: Vec<FxHashSet<String>> = out.iter().map(|e| {
        first_content(&e.body).split_whitespace()
            .filter(|w| w.len() >= 3).map(|w| w.to_lowercase()).collect()
    }).collect();
    supersede(&mut out, &tokens);
    temporal_chains(&mut out, &tokens);
    out.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// First non-metadata content line of an entry body.
pub fn first_content(body: &str) -> &str {
    body.lines().find(|l| {
        let t = l.trim();
        !t.is_empty() && !crate::text::is_metadata_line(t)
    }).unwrap_or("")
}

/// Cross-topic dedup: identical first content lines → merge with provenance.
fn dedup(entries: &mut Vec<Compressed>) {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    // Pre-compute lowercased first-content keys once instead of per-comparison
    let keys: Vec<String> = entries.iter()
        .map(|e| first_content(&e.body).to_lowercase()).collect();
    for (i, key) in keys.iter().enumerate() {
        if key.len() >= 10 { groups.entry(key.clone()).or_default().push(i); }
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

/// Supersession: same topic, >60% first_content overlap, >1 day apart → dim older.
/// Now visible: chain text identifies the superseding entry.
/// Uses shared FxHashSet tokens for O(1) intersection lookups.
fn supersede(entries: &mut [Compressed], tokens: &[FxHashSet<String>]) {
    let mut by_topic: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        by_topic.entry(e.topic.as_str()).or_default().push(i);
    }
    // Track: dimmed_idx → newer_idx (the entry that supersedes it)
    let mut superseded_by: BTreeMap<usize, usize> = BTreeMap::new();
    for (_, indices) in &by_topic {
        for (a, &i) in indices.iter().enumerate() {
            if tokens[i].len() < 3 || superseded_by.contains_key(&i) { continue; }
            for &j in &indices[a+1..] {
                if tokens[j].len() < 3 || superseded_by.contains_key(&j) { continue; }
                let isect = tokens[i].iter().filter(|t| tokens[j].contains(t.as_str())).count();
                let union = tokens[i].len() + tokens[j].len() - isect;
                if union == 0 || isect * 100 / union < 60 { continue; }
                if (entries[i].days_old - entries[j].days_old).abs() < 2 { continue; }
                if entries[i].days_old > entries[j].days_old {
                    superseded_by.insert(i, j);
                } else {
                    superseded_by.insert(j, i);
                }
            }
        }
    }
    for (&dimmed, &newer) in &superseded_by {
        entries[dimmed].relevance *= 0.5;
        let newer_fc = first_content(&entries[newer].body);
        let preview = crate::text::truncate(newer_fc, 50);
        entries[dimmed].chain = Some(format!("superseded by: {}", preview));
    }
}

/// Temporal chains: same topic + same dominant entity → compress to timeline.
/// Uses shared FxHashSet tokens for O(1) Jaccard intersection in pass 3.
fn temporal_chains(entries: &mut Vec<Compressed>, tokens: &[FxHashSet<String>]) {
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(term) = dominant_term(first_content(&e.body)) {
            groups.entry((e.topic.clone(), term)).or_default().push(i);
        }
    }
    let mut remove = Vec::new();
    // Pass 1: dominant term grouping
    for ((_, term), indices) in &groups {
        if indices.len() < 2 { continue; }
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
    // Early exit: if >60% of entries are already chained, skip remaining passes
    let chained_pct = if entries.is_empty() { 0 } else { remove.len() * 100 / entries.len() };
    if chained_pct > 60 {
        remove.sort_unstable();
        remove.dedup();
        for &idx in remove.iter().rev() { entries.remove(idx); }
        return;
    }
    // Pass 2: date-proximity fallback — group unchained same-topic entries
    // within 48-hour buckets. Only chains groups of 3+ (avoids trivial pairs).
    let chained: std::collections::BTreeSet<usize> = remove.iter().copied()
        .chain(entries.iter().enumerate()
            .filter(|(_, e)| e.chain.is_some()).map(|(i, _)| i))
        .collect();
    let mut date_groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        if chained.contains(&i) { continue; }
        let bucket = e.days_old / 2;
        date_groups.entry(format!("{}:{}", e.topic, bucket)).or_default().push(i);
    }
    for (_, indices) in &date_groups {
        if indices.len() < 3 { continue; }
        let mut sorted: Vec<usize> = indices.clone();
        sorted.sort_by(|a, b| entries[*b].days_old.cmp(&entries[*a].days_old));
        let mut labels: Vec<String> = Vec::new();
        let mut seen = FxHashSet::default();
        for &i in sorted.iter().take(4) {
            let lbl = label_words(first_content(&entries[i].body), 3);
            if !lbl.is_empty() && seen.insert(lbl.clone()) { labels.push(lbl); }
        }
        let date = &entries[sorted[0]].date;
        let date_short = if date.len() >= 10 { &date[..10] } else { date };
        let chain = format!("batch {}: {}", date_short, labels.join(" | "));
        let newest = *sorted.last().unwrap();
        entries[newest].chain = Some(chain);
        entries[newest].relevance += sorted.len() as f64;
        for &idx in sorted.iter().take(sorted.len() - 1) { remove.push(idx); }
    }
    // Pass 3: token-similarity grouping — reuses shared tokens (FxHashSet, O(1) lookups)
    let chained2: std::collections::BTreeSet<usize> = remove.iter().copied()
        .chain(entries.iter().enumerate()
            .filter(|(_, e)| e.chain.is_some()).map(|(i, _)| i))
        .collect();
    let sim_groups: Vec<Vec<usize>> = {
        let mut topic_unchained: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, e) in entries.iter().enumerate() {
            if chained2.contains(&i) { continue; }
            topic_unchained.entry(e.topic.as_str()).or_default().push(i);
        }
        let mut all_groups = Vec::new();
        for (_, indices) in &topic_unchained {
            if indices.len() < 2 { continue; }
            // Cap pairwise comparisons to avoid O(N²) on large topic groups
            let capped = if indices.len() > 50 { &indices[..50] } else { &indices[..] };
            let mut sim: Vec<Vec<usize>> = Vec::new();
            for &i in capped {
                let mut found = false;
                for g in &mut sim {
                    let j = g[0];
                    let isect = tokens[i].iter().filter(|t| tokens[j].contains(t.as_str())).count();
                    let union = tokens[i].len() + tokens[j].len() - isect;
                    if union > 0 && isect * 100 / union >= 40 {
                        g.push(i); found = true; break;
                    }
                }
                if !found { sim.push(vec![i]); }
            }
            for g in sim { if g.len() >= 2 { all_groups.push(g); } }
        }
        all_groups
    };
    for mut g in sim_groups {
        g.sort_by(|a, b| entries[*b].days_old.cmp(&entries[*a].days_old));
        let mut labels: Vec<String> = Vec::new();
        let mut seen = FxHashSet::default();
        for &i in g.iter().take(3) {
            let lbl = label_words(first_content(&entries[i].body), 3);
            if !lbl.is_empty() && seen.insert(lbl.clone()) { labels.push(lbl); }
        }
        let chain = format!("similar: {}", labels.join(" | "));
        let newest = *g.last().unwrap();
        entries[newest].chain = Some(chain);
        entries[newest].relevance += g.len() as f64;
        for &idx in g.iter().take(g.len() - 1) { remove.push(idx); }
    }
    remove.sort_unstable();
    remove.dedup();
    for &idx in remove.iter().rev() { entries.remove(idx); }
}

/// Extract a short label from a content line. Better than char-truncation
/// which cuts mid-word producing unreadable fragments.
/// Strategy: take first 3 meaningful words, stop at parens/brackets.
fn label_words(line: &str, n: usize) -> String {
    let cleaned = line.trim_start_matches(|c: char| c == '#' || c == '*' || c == '-' || c == ' ');
    let mut words = Vec::new();
    for w in cleaned.split_whitespace() {
        // Stop at structural noise: parens, long paths, arrows, dashes
        if w.starts_with('(') || w.starts_with('[') || w.contains('/')
            || w == "→" || w == "--" || w == "—" {
            break;
        }
        words.push(w);
        if words.len() >= n { break; }
    }
    let label = words.join(" ");
    // Strip trailing punctuation that looks orphaned
    label.trim_end_matches(|c: char| c == ':' || c == ',' || c == ';' || c == '—').to_string()
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
