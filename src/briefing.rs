//! v5 LLM-native briefing formatter. Takes compressed entries, produces
//! hierarchical output: topic map → graph → categories → untagged → gaps → stats.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use crate::compress::{Compressed, first_content};

const CATEGORIES: &[(&str, &[&str])] = &[
    ("ARCHITECTURE", &["architecture", "module-map", "overview", "dependency-graph"]),
    ("DATA FLOW", &["pipeline", "data-flow"]),
    ("INVARIANTS", &["invariant", "constraint", "limit"]),
    ("CHANGE IMPACT", &["change-impact"]),
    ("GOTCHAS", &["gotcha", "tf-mismatch", "timestamp-loss"]),
    ("DECISIONS", &["decision"]),
    ("HOW-TO", &["how-to", "workflow", "add-tool"]),
    ("SCORING & SEARCH", &["bm25", "scoring", "algorithm", "query-parsing"]),
    ("DATA FORMAT", &["dataformat", "binary-format", "data-log", "index-bin"]),
    ("PERFORMANCE", &["performance", "slow-path", "zero-alloc", "data-structure"]),
    ("API SURFACE", &["api-surface", "tool", "schema", "mcp-api", "variant"]),
];
const CORE_TAGS: &[&str] = &["architecture", "data-flow", "invariant", "change-impact"];

pub fn format(entries: &[Compressed], query: &str, raw_count: usize,
              primary: &[String]) -> String {
    let n_topics = entries.iter().map(|e| e.topic.as_str())
        .collect::<BTreeSet<_>>().len();
    let mut out = String::new();
    let _ = writeln!(out, "=== {} === {} entries → {} compressed, {} topics\n",
        query.to_uppercase(), raw_count, entries.len(), n_topics);
    write_topics(&mut out, entries, primary);
    write_graph(&mut out, entries, primary);
    let used = write_categories(&mut out, entries);
    write_untagged(&mut out, entries, &used, primary);
    write_gaps(&mut out, entries, primary);
    write_stats(&mut out, entries, raw_count);
    out
}

fn write_topics(out: &mut String, entries: &[Compressed], primary: &[String]) {
    let mut info: BTreeMap<&str, (usize, i64)> = BTreeMap::new();
    for e in entries {
        let (count, newest) = info.entry(&e.topic).or_insert((0, i64::MAX));
        *count += 1;
        if e.days_old < *newest { *newest = e.days_old; }
    }
    let _ = write!(out, "TOPICS:");
    for t in primary {
        if let Some((c, d)) = info.get(t.as_str()) {
            let _ = write!(out, " {} ({}{})", t, c, freshness_short(*d));
        }
    }
    let _ = writeln!(out, "\n");
}

fn write_graph(out: &mut String, entries: &[Compressed], primary: &[String]) {
    if primary.len() < 2 { return; }
    let mut edges: Vec<(String, String, usize)> = Vec::new();
    for src in primary {
        for tgt in primary {
            if src == tgt { continue; }
            let refs: usize = entries.iter()
                .filter(|e| e.topic == *src)
                .map(|e| count_ci(&e.body, tgt))
                .sum();
            if refs > 0 { edges.push((src.clone(), tgt.clone(), refs)); }
        }
    }
    edges.sort_by(|a, b| b.2.cmp(&a.2));
    if !edges.is_empty() {
        let _ = write!(out, "GRAPH:");
        for (s, t, n) in edges.iter().take(6) {
            let _ = write!(out, " {} → {} ({})", s, t, n);
        }
        let _ = writeln!(out, "\n");
    }
}

fn write_categories(out: &mut String, entries: &[Compressed]) -> Vec<usize> {
    let mut used = Vec::new();
    for &(cat, patterns) in CATEGORIES {
        let group: Vec<(usize, &Compressed)> = entries.iter().enumerate()
            .filter(|(i, e)| !used.contains(i)
                && e.tags.iter().any(|t| patterns.contains(&t.as_str())))
            .collect();
        if group.is_empty() { continue; }
        for &(i, _) in &group { used.push(i); }
        let _ = writeln!(out, "--- {} ({}) ---", cat, group.len());
        // Top 5 in full, next 10 as one-liners, rest summarized
        for (_, e) in group.iter().take(5) { format_entry(out, e); }
        let rest = group.len().saturating_sub(5);
        let oneliners = rest.min(10);
        for (_, e) in group.iter().skip(5).take(oneliners) { format_oneliner(out, e); }
        if rest > oneliners {
            let _ = writeln!(out, "  ... +{} more {} entries\n", rest - oneliners,
                cat.to_lowercase());
        }
    }
    used
}

fn write_untagged(out: &mut String, entries: &[Compressed], used: &[usize],
                  primary: &[String]) {
    let untagged: Vec<&Compressed> = entries.iter().enumerate()
        .filter(|(i, _)| !used.contains(i)).map(|(_, e)| e).collect();
    if untagged.is_empty() { return; }

    // Group by topic, budget: primary=5, non-primary=2
    let mut by_topic: BTreeMap<&str, Vec<&Compressed>> = BTreeMap::new();
    for e in &untagged { by_topic.entry(&e.topic).or_default().push(e); }
    for group in by_topic.values_mut() {
        group.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal));
    }
    let mut shown = 0usize;
    let mut hidden = 0usize;
    let _ = writeln!(out, "--- UNTAGGED ({}) ---", untagged.len());
    for (topic, group) in &by_topic {
        let budget = if primary.iter().any(|p| p == topic) { 5 } else { 2 };
        for e in group.iter().take(budget) {
            format_oneliner(out, e);
            shown += 1;
        }
        if group.len() > budget {
            let extra = group.len() - budget;
            let _ = writeln!(out, "  [{}] ... +{} more entries", topic, extra);
            hidden += extra;
        }
    }
    if hidden > 0 {
        let _ = writeln!(out, "  ({} shown, {} compressed away)", shown, hidden);
    }
    let _ = writeln!(out);
}

fn write_gaps(out: &mut String, entries: &[Compressed], primary: &[String]) {
    let mut suggestions: Vec<String> = Vec::new();
    for topic in primary {
        let count = entries.iter().filter(|e| e.topic == *topic).count();
        if count < 10 { continue; }
        let topic_tags: BTreeSet<&str> = entries.iter()
            .filter(|e| e.topic == *topic)
            .flat_map(|e| e.tags.iter().map(|t| t.as_str()))
            .collect();
        for &core in CORE_TAGS {
            if !topic_tags.contains(core) {
                suggestions.push(format!(
                    "  store topic=\"{}\" tags=\"{}\" text=\"TODO: {} for {}\"",
                    topic, core, core, topic));
            }
        }
    }
    if !suggestions.is_empty() {
        let _ = writeln!(out, "GAPS ({} missing core tags):", suggestions.len());
        for s in &suggestions { let _ = writeln!(out, "{}", s); }
        let _ = writeln!(out);
    }
}

fn write_stats(out: &mut String, entries: &[Compressed], raw_count: usize) {
    let tagged = entries.iter().filter(|e| !e.tags.is_empty()).count();
    let sourced = entries.iter().filter(|e| e.source.is_some()).count();
    let chained = entries.iter().filter(|e| e.chain.is_some()).count();
    let pct = if raw_count > 0 { 100 - (entries.len() * 100 / raw_count) } else { 0 };
    let _ = writeln!(out, "STATS: {} entries, {} tagged, {} sourced, {} chained | compressed {}→{} ({}% reduction)",
        entries.len(), tagged, sourced, chained, raw_count, entries.len(), pct);
}

fn format_entry(out: &mut String, e: &Compressed) {
    let src = e.source.as_deref().map(|s| format!(" → {s}")).unwrap_or_default();
    let also = format_also(&e.also_in);
    let chain_note = e.chain.as_deref().map(|_| " (chained)").unwrap_or("");
    let _ = writeln!(out, "[{}] {}{}{}{}{}", e.topic, e.date, freshness_tag(e.days_old),
        src, also, chain_note);
    if let Some(ref chain) = e.chain {
        let _ = writeln!(out, "  {}", crate::text::truncate(chain, 120));
    }
    let lines: Vec<&str> = e.body.lines()
        .filter(|l| !l.starts_with("[tags:") && !l.starts_with("[source:")
            && !l.starts_with("[type:") && !l.starts_with("[modified:")
            && !l.starts_with("[tier:"))
        .collect();
    for l in lines.iter().take(5) { let _ = writeln!(out, "  {}", l.trim()); }
    if lines.len() > 5 { let _ = writeln!(out, "  ...({} more lines)", lines.len() - 5); }
    let _ = writeln!(out);
}

fn format_oneliner(out: &mut String, e: &Compressed) {
    let fc = crate::text::truncate(first_content(&e.body), 80);
    let src = e.source.as_deref().map(|s| format!(" → {s}")).unwrap_or_default();
    let also = format_also(&e.also_in);
    let chain = e.chain.as_deref().map(|c| format!(" ({})", c)).unwrap_or_default();
    let _ = writeln!(out, "  [{}] {}{}{}{}{}", e.topic, fc, src, also, chain,
        freshness_tag(e.days_old));
}

fn format_also(topics: &[String]) -> String {
    if topics.is_empty() { return String::new(); }
    let deduped: BTreeSet<&str> = topics.iter().map(|s| s.as_str()).collect();
    let items: Vec<&str> = deduped.iter().copied().take(3).collect();
    let extra = if deduped.len() > 3 { format!("+{}", deduped.len() - 3) } else { String::new() };
    format!(" [also: {}{}]", items.join(", "), extra)
}

fn freshness_tag(days: i64) -> &'static str {
    match days { 0 => " [TODAY]", 1 => " [1d]", 2..=7 => " [week]", _ => "" }
}

fn freshness_short(days: i64) -> &'static str {
    match days { 0 => ", today", 1 => ", 1d", 2..=7 => ", week", _ => "" }
}

/// Count case-insensitive substring occurrences without allocation.
/// Needle must be ASCII lowercase (topic names always are).
fn count_ci(haystack: &str, needle: &str) -> usize {
    let nb = needle.as_bytes();
    if nb.is_empty() || nb.len() > haystack.len() { return 0; }
    haystack.as_bytes().windows(nb.len())
        .filter(|w| w.iter().zip(nb).all(|(h, n)| h.to_ascii_lowercase() == *n))
        .count()
}
