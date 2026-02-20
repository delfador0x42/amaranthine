//! v7.2 Reconstruct: one-shot compressed briefing with tiered output.
//! Supports glob patterns (iris-*), temporal filtering (since=24h),
//! source-path matching (cache.rs → entries with [source: ...cache.rs]),
//! focus filtering (focus=gotchas,invariants → only those categories),
//! and three detail levels (summary/scan/full).

use std::collections::BTreeSet;
use std::path::Path;
use crate::compress::RawEntry;
use crate::fxhash::{FxHashMap, FxHashSet};

pub fn run(dir: &Path, query: &str, detail: &str, since_hours: Option<u64>,
           focus: Option<&str>) -> Result<String, String> {
    let q = query.to_lowercase();
    let is_glob = q.contains('*');
    let is_source_query = query.contains('.') && !query.contains(' ');
    let q_sanitized = if is_glob { q.clone() } else { crate::config::sanitize_topic(query) };
    let q_terms = crate::text::query_terms(query);
    let now_days = crate::time::LocalTime::now().to_days();
    let max_days = since_hours.map(|h| if h <= 12 { 0i64 } else { (h as i64 - 1) / 24 });

    // Parse focus categories (comma-separated, case-insensitive)
    let focus_cats: Option<Vec<String>> = focus.map(|f|
        f.split(',').map(|c| c.trim().to_uppercase()).filter(|c| !c.is_empty()).collect()
    );

    crate::cache::with_corpus(dir, |cached| {
        // Identify primary topics (glob or substring match)
        let mut primary_set: BTreeSet<&str> = BTreeSet::new();
        for e in cached {
            let topic = e.topic.as_str();
            if is_glob {
                if glob_match(&q, topic) { primary_set.insert(topic); }
            } else if !is_source_query {
                if topic.contains(q_sanitized.as_str()) { primary_set.insert(topic); }
            }
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
                for (lt, li) in e.links() {
                    *link_in_counts.entry(link_key(lt, *li)).or_default() += 1;
                }
            }
        }

        for e in cached {
            let is_primary = primary_set.contains(e.topic.as_str());
            let is_related = !q_terms.is_empty()
                && q_terms.iter().any(|t| e.tf_map.contains_key(t));
            // Source-path matching: find entries whose [source:] contains the query
            let is_source_match = is_source_query && e.source()
                .map_or(false, |s| source_matches(s, query));

            if !is_primary && !is_related && !is_source_match { continue; }
            let days_old = e.days_old(now_days);
            // --since filter: skip entries older than cutoff
            if let Some(max) = max_days {
                if days_old > max { continue; }
            }
            matched_offsets.insert(e.offset);
            let mut relevance = if is_primary { 10.0 }
                else if is_source_match { 15.0 } // source matches rank highest
                else { 0.0 };
            for t in &q_terms {
                relevance += *e.tf_map.get(t).unwrap_or(&0) as f64;
            }
            // Freshness boost (stable knowledge exempt)
            if !e.has_tag("invariant") && !e.has_tag("architecture") {
                relevance *= 1.0 + 1.0 / (1.0 + days_old as f64 / 7.0);
            }
            relevance *= e.confidence();
            let tidx = offset_tidx.get(&e.offset).copied().unwrap_or(0);
            let link_in = link_in_counts.get(&link_key(e.topic.as_str(), tidx))
                .copied().unwrap_or(0);
            relevance += link_in as f64 * 2.0;

            // If source query matched, also add the topic as primary for display
            if is_source_match && !primary_set.contains(e.topic.as_str()) {
                primary_set.insert(e.topic.as_str());
            }

            entries.push(RawEntry {
                topic: e.topic.to_string(), body: e.body.clone(),
                timestamp_min: e.timestamp_min, days_old,
                tags: e.tags().to_vec(), relevance,
                confidence: e.confidence(), link_in,
            });
        }

        // Follow narrative links (1 level) — skip when --since is active
        if max_days.is_none() {
            let has_any_links = cached.iter()
                .any(|e| !e.links().is_empty() && matched_offsets.contains(&e.offset));
            if has_any_links {
                let mut topic_idx_map: std::collections::BTreeMap<(&str, usize), usize> = std::collections::BTreeMap::new();
                let mut topic_counters: FxHashMap<&str, usize> = FxHashMap::default();
                for (pos, e) in cached.iter().enumerate() {
                    let idx = topic_counters.entry(e.topic.as_str()).or_default();
                    topic_idx_map.insert((e.topic.as_str(), *idx), pos);
                    *idx += 1;
                }
                for e in cached {
                    if !matched_offsets.contains(&e.offset) || e.links().is_empty() { continue; }
                    for (link_topic, link_idx) in e.links() {
                        if let Some(&pos) = topic_idx_map.get(&(link_topic.as_str(), *link_idx)) {
                            let le = &cached[pos];
                            if !matched_offsets.contains(&le.offset) {
                                let days_old = le.days_old(now_days);
                                let le_tidx = offset_tidx.get(&le.offset).copied().unwrap_or(0);
                                let le_link_in = link_in_counts.get(&link_key(le.topic.as_str(), le_tidx))
                                    .copied().unwrap_or(0);
                                entries.push(RawEntry {
                                    topic: le.topic.to_string(),
                                    body: format!("[linked from: {}:{}]\n{}", e.topic, link_idx, le.body),
                                    timestamp_min: le.timestamp_min, days_old,
                                    tags: le.tags().to_vec(),
                                    relevance: 3.0 * le.confidence(),
                                    confidence: le.confidence(), link_in: le_link_in,
                                });
                                matched_offsets.insert(le.offset);
                            }
                        }
                    }
                }
            }
        }

        if entries.is_empty() {
            return if since_hours.is_some() {
                format!("No new entries for '{}' in the last {}h.\n", query, since_hours.unwrap())
            } else {
                format!("No entries found for '{query}'.\n")
            };
        }

        let primary: Vec<String> = primary_set.iter().map(|s| s.to_string()).collect();
        let raw_count = entries.len();
        let compressed = crate::compress::compress(entries);
        let d = crate::briefing::Detail::from_str(detail);
        crate::briefing::format(&compressed, query, raw_count, &primary, d, since_hours,
                                focus_cats.as_deref())
    })
}

/// Check if a [source:] path matches a query file name.
/// "src/cache.rs:11" matches query "cache.rs"
/// "amaranthine/src/mcp.rs:1" matches query "mcp.rs"
fn source_matches(source: &str, query: &str) -> bool {
    let path = source.split(':').next().unwrap_or(source);
    // Exact filename match (most common)
    if path.ends_with(query) {
        let prefix_end = path.len() - query.len();
        prefix_end == 0 || path.as_bytes()[prefix_end - 1] == b'/'
    } else {
        false
    }
}

/// Simple glob matching: supports * wildcards.
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 { return text.contains(pattern); }
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() { continue; }
        if i == 0 {
            if !text.starts_with(part) { return false; }
            pos = part.len();
        } else if i == parts.len() - 1 {
            if !text[pos..].ends_with(part) { return false; }
        } else {
            match text[pos..].find(part) {
                Some(idx) => pos += idx + part.len(),
                None => return false,
            }
        }
    }
    true
}

/// FNV-1a hash of (topic, idx) pair for link-in counting. Zero allocation.
fn link_key(topic: &str, idx: usize) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in topic.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); }
    h ^= idx as u64;
    h = h.wrapping_mul(0x100000001b3);
    h
}
