//! v5 Reconstruct: one-shot compressed briefing.
//! Uses corpus cache → compress → format LLM-native output.

use std::collections::BTreeSet;
use std::path::Path;
use crate::compress::RawEntry;

pub fn run(dir: &Path, query: &str) -> Result<String, String> {
    let q = crate::config::sanitize_topic(query);
    let q_terms = crate::text::query_terms(query);
    let now_days = crate::time::LocalTime::now().to_days();

    crate::cache::with_corpus(dir, |cached| {
        // Identify primary topics (name contains query)
        let mut topic_set = BTreeSet::new();
        for e in cached { topic_set.insert(e.topic.as_str()); }
        let primary: Vec<String> = topic_set.iter()
            .filter(|n| n.contains(q.as_str()))
            .map(|s| s.to_string()).collect();

        // Collect matching entries using pre-tokenized token_set
        let mut entries: Vec<RawEntry> = Vec::new();
        for e in cached {
            let is_primary = primary.iter().any(|p| e.topic == p.as_str());
            let is_related = !q_terms.is_empty()
                && q_terms.iter().any(|t| e.token_set.contains(t));
            if !is_primary && !is_related { continue; }
            let days_old = e.days_old(now_days);
            let mut relevance = if is_primary { 10.0 } else { 0.0 };
            // Count term hits via tf_map instead of body.to_lowercase().matches()
            for t in &q_terms {
                relevance += *e.tf_map.get(t).unwrap_or(&0) as f64;
            }
            entries.push(RawEntry {
                topic: e.topic.to_string(), body: e.body.clone(),
                timestamp_min: e.timestamp_min, days_old,
                tags: e.tags().iter().map(|t| t.to_string()).collect(), relevance,
            });
        }

        // Follow narrative links (1 level): pull in linked entries not already collected
        let mut linked_offsets: BTreeSet<u32> = entries.iter()
            .map(|_| 0u32).collect(); // placeholder — use actual offsets below
        linked_offsets.clear();
        for e in cached {
            let dominated = primary.iter().any(|p| e.topic == p.as_str())
                || (!q_terms.is_empty() && q_terms.iter().any(|t| e.token_set.contains(t)));
            if dominated { linked_offsets.insert(e.offset); }
        }
        for e in cached {
            if !linked_offsets.contains(&e.offset) { continue; }
            if e.links.is_empty() { continue; }
            for (link_topic, link_idx) in &e.links {
                // Find the linked entry in cached by topic + index
                let mut idx = 0usize;
                for le in cached {
                    if le.topic.as_str() != link_topic { continue; }
                    if idx == *link_idx {
                        if !linked_offsets.contains(&le.offset) {
                            let days_old = le.days_old(now_days);
                            entries.push(RawEntry {
                                topic: le.topic.to_string(),
                                body: format!("[linked from: {}:{}]\n{}", e.topic, link_idx, le.body),
                                timestamp_min: le.timestamp_min, days_old,
                                tags: le.tags().iter().map(|t| t.to_string()).collect(),
                                relevance: 3.0, // linked entries get moderate relevance
                            });
                            linked_offsets.insert(le.offset);
                        }
                        break;
                    }
                    idx += 1;
                }
            }
        }

        if entries.is_empty() { return format!("No entries found for '{query}'.\n"); }

        let raw_count = entries.len();
        let compressed = crate::compress::compress(entries);
        crate::briefing::format(&compressed, query, raw_count, &primary)
    })
}
