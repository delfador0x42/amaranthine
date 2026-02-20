//! v5 Reconstruct: one-shot compressed briefing.
//! Uses corpus cache → compress → format LLM-native output.

use std::collections::BTreeSet;
use std::path::Path;
use crate::compress::RawEntry;
use crate::fxhash::{FxHashMap, FxHashSet};

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

        // Quality signals: link-in counts + offset→topic_idx
        let mut link_in_counts: FxHashMap<u64, u16> = FxHashMap::default();
        let mut offset_tidx: FxHashMap<u32, usize> = FxHashMap::default();
        {
            let mut counters: FxHashMap<&str, usize> = FxHashMap::default();
            for e in cached {
                let idx = counters.entry(e.topic.as_str()).or_default();
                offset_tidx.insert(e.offset, *idx);
                *idx += 1;
            }
            for e in cached {
                for (lt, li) in &e.links {
                    *link_in_counts.entry(link_key(lt, *li)).or_default() += 1;
                }
            }
        }

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
            // Freshness boost (stable knowledge exempt)
            if !e.has_tag("invariant") && !e.has_tag("architecture") {
                relevance *= 1.0 + 1.0 / (1.0 + days_old as f64 / 7.0);
            }
            relevance *= e.confidence;
            let tidx = offset_tidx.get(&e.offset).copied().unwrap_or(0);
            let link_in = link_in_counts.get(&link_key(e.topic.as_str(), tidx))
                .copied().unwrap_or(0);
            relevance += link_in as f64 * 2.0;
            entries.push(RawEntry {
                topic: e.topic.to_string(), body: e.body.clone(),
                timestamp_min: e.timestamp_min, days_old,
                tags: e.tags.clone(), relevance,
                confidence: e.confidence, link_in,
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
                                let le_tidx = offset_tidx.get(&le.offset).copied().unwrap_or(0);
                                let le_link_in = link_in_counts.get(&link_key(le.topic.as_str(), le_tidx))
                                    .copied().unwrap_or(0);
                                entries.push(RawEntry {
                                    topic: le.topic.to_string(),
                                    body: format!("[linked from: {}:{}]\n{}", e.topic, link_idx, le.body),
                                    timestamp_min: le.timestamp_min, days_old,
                                    tags: le.tags.clone(),
                                    relevance: 3.0 * le.confidence,
                                    confidence: le.confidence, link_in: le_link_in,
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

/// FNV-1a hash of (topic, idx) pair for link-in counting. Zero allocation.
fn link_key(topic: &str, idx: usize) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in topic.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); }
    h ^= idx as u64;
    h = h.wrapping_mul(0x100000001b3);
    h
}
