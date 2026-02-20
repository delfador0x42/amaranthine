//! v5 Reconstruct: one-shot compressed briefing.
//! Uses corpus cache → compress → format LLM-native output.

use std::collections::BTreeSet;
use std::path::Path;
use crate::compress::RawEntry;
use crate::fxhash::FxHashSet;

pub fn run(dir: &Path, query: &str) -> Result<String, String> {
    let q = crate::config::sanitize_topic(query);
    let q_terms = crate::text::query_terms(query);
    let now_days = crate::time::LocalTime::now().to_days();

    crate::cache::with_corpus(dir, |cached| {
        // Single pass: identify primary topics + collect matching entries + track offsets
        let mut primary_set: BTreeSet<&str> = BTreeSet::new();
        for e in cached {
            if e.topic.contains(q.as_str()) { primary_set.insert(e.topic.as_str()); }
        }

        let mut entries: Vec<RawEntry> = Vec::new();
        let mut matched_offsets: FxHashSet<u32> = FxHashSet::default();

        for e in cached {
            let is_primary = primary_set.contains(e.topic.as_str());
            let is_related = !q_terms.is_empty()
                && q_terms.iter().any(|t| e.tf_map.contains_key(t));
            if !is_primary && !is_related { continue; }
            matched_offsets.insert(e.offset);
            let days_old = e.days_old(now_days);
            let mut relevance = if is_primary { 10.0 } else { 0.0 };
            for t in &q_terms {
                relevance += *e.tf_map.get(t).unwrap_or(&0) as f64;
            }
            entries.push(RawEntry {
                topic: e.topic.to_string(), body: e.body.clone(),
                timestamp_min: e.timestamp_min, days_old,
                tags: e.tags.clone(), relevance,
            });
        }

        // Follow narrative links (1 level): pull in linked entries not already collected
        // Only scan entries that have links (skip the common case of no links)
        let has_any_links = cached.iter().any(|e| !e.links.is_empty() && matched_offsets.contains(&e.offset));
        if has_any_links {
            for e in cached {
                if !matched_offsets.contains(&e.offset) || e.links.is_empty() { continue; }
                for (link_topic, link_idx) in &e.links {
                    let mut idx = 0usize;
                    for le in cached {
                        if le.topic.as_str() != link_topic { continue; }
                        if idx == *link_idx {
                            if !matched_offsets.contains(&le.offset) {
                                let days_old = le.days_old(now_days);
                                entries.push(RawEntry {
                                    topic: le.topic.to_string(),
                                    body: format!("[linked from: {}:{}]\n{}", e.topic, link_idx, le.body),
                                    timestamp_min: le.timestamp_min, days_old,
                                    tags: le.tags.clone(),
                                    relevance: 3.0,
                                });
                                matched_offsets.insert(le.offset);
                            }
                            break;
                        }
                        idx += 1;
                    }
                }
            }
        }

        if entries.is_empty() { return format!("No entries found for '{query}'.\n"); }

        let primary: Vec<String> = primary_set.iter().map(|s| s.to_string()).collect();
        let raw_count = entries.len();
        let compressed = crate::compress::compress(entries);
        crate::briefing::format(&compressed, query, raw_count, &primary)
    })
}
