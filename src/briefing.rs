//! v7 LLM-native briefing formatter with tiered output.
//! Summary (default): category counts + hot items (~15 lines)
//! Scan: categories with one-liners (~50 lines)
//! Full: categories with full entries (current behavior)

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use crate::compress::{Compressed, first_content};

pub enum Detail { Summary, Scan, Full }

impl Detail {
    pub fn from_str(s: &str) -> Self {
        match s { "scan" => Detail::Scan, "full" => Detail::Full, _ => Detail::Summary }
    }
}

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
    ("GAPS", &["gap", "missing"]),
];

/// Content prefixes: map entry opening lines to categories via starts_with.
/// Catches entries that have the right structure but lack proper tags.
const CONTENT_PREFIXES: &[(&str, &[&str])] = &[
    ("DATA FLOW", &["flow:", "data flow:"]),
    ("INVARIANTS", &["security:", "invariant:"]),
    ("GOTCHAS", &["deploy gotcha:"]),
    ("DECISIONS", &["design:", "architectural decision:"]),
    ("GAPS", &["friction", "gap:", "todo:", "missing:"]),
    ("HOW-TO", &["shipped", "impl spec:", "impl:"]),
    ("PERFORMANCE", &["perf:", "benchmark:"]),
];

const CORE_TAGS: &[&str] = &["architecture", "data-flow", "invariant", "change-impact"];

// --- Classification ---

struct Classification {
    structural: Vec<usize>,
    categories: Vec<(&'static str, Vec<usize>)>,
    dynamic: Vec<(String, Vec<usize>)>,
    untagged: Vec<usize>,
}

fn classify(entries: &[Compressed]) -> Classification {
    let fc_lower: Vec<String> = entries.iter()
        .map(|e| first_content(&e.body).to_lowercase()).collect();
    let mut assigned = vec![false; entries.len()];

    // Pass 1: structural (raw-data + structural/coupling/callgraph)
    let mut structural = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if e.tags.iter().any(|t| t == "raw-data")
            && e.tags.iter().any(|t| t == "structural" || t == "coupling" || t == "callgraph")
        {
            structural.push(i);
            assigned[i] = true;
        }
    }

    // Pass 2: static categories (tag match + keyword match + content-prefix match)
    let mut categories: Vec<(&'static str, Vec<usize>)> = Vec::new();
    for &(cat, patterns) in CATEGORIES {
        let mut group = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            if assigned[i] || e.tags.iter().any(|t| t == "raw-data") { continue; }
            let tag_match = e.tags.iter().any(|t| patterns.contains(&t.as_str()));
            let keyword_match = patterns.iter().any(|p| fc_lower[i].contains(p));
            let prefix_match = CONTENT_PREFIXES.iter()
                .find(|(c, _)| *c == cat)
                .map_or(false, |(_, prefs)|
                    prefs.iter().any(|p| fc_lower[i].starts_with(p)));
            if tag_match || keyword_match || prefix_match {
                group.push(i);
                assigned[i] = true;
            }
        }
        if !group.is_empty() { categories.push((cat, group)); }
    }

    // Pass 3: dynamic categories (unclaimed tags with 3+ entries)
    let static_tags: BTreeSet<&str> = CATEGORIES.iter()
        .flat_map(|(_, pats)| pats.iter().copied()).collect();
    let mut tag_freq: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        if assigned[i] || e.tags.iter().any(|t| t == "raw-data") { continue; }
        for t in &e.tags {
            if !static_tags.contains(t.as_str()) {
                tag_freq.entry(t.as_str()).or_default().push(i);
            }
        }
    }
    let mut raw_dynamic: Vec<(&str, Vec<usize>)> = tag_freq.into_iter()
        .filter(|(_, v)| v.len() >= 3).collect();
    raw_dynamic.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    raw_dynamic.truncate(5);
    let mut dynamic: Vec<(String, Vec<usize>)> = Vec::new();
    for (tag, indices) in raw_dynamic {
        let clean: Vec<usize> = indices.into_iter()
            .filter(|i| !assigned[*i]).collect();
        if clean.is_empty() { continue; }
        for &i in &clean { assigned[i] = true; }
        dynamic.push((tag.to_string(), clean));
    }

    // Remaining → untagged
    let untagged: Vec<usize> = (0..entries.len())
        .filter(|i| !assigned[*i] && !entries[*i].tags.iter().any(|t| t == "raw-data"))
        .collect();

    Classification { structural, categories, dynamic, untagged }
}

// --- Public entry point ---

pub fn format(entries: &[Compressed], query: &str, raw_count: usize,
              primary: &[String], detail: Detail, since: Option<u64>) -> String {
    match detail {
        Detail::Summary => format_summary(entries, query, raw_count, primary, since),
        Detail::Scan => format_scan(entries, query, raw_count, primary, since),
        Detail::Full => format_full(entries, query, raw_count, primary, since),
    }
}

// --- Tier 1: Summary (~15 lines) ---

fn format_summary(entries: &[Compressed], query: &str, raw_count: usize,
                  primary: &[String], since: Option<u64>) -> String {
    let cls = classify(entries);
    let n_topics = entries.iter().map(|e| e.topic.as_str())
        .collect::<BTreeSet<_>>().len();
    let mut out = String::new();

    // Header
    let since_note = since.map(|h| format!(" (since {}h)", h)).unwrap_or_default();
    let _ = writeln!(out, "=== {}{} === {} entries \u{2192} {} compressed, {} topics\n",
        query.to_uppercase(), since_note, raw_count, entries.len(), n_topics);

    // Topics
    write_topics_brief(&mut out, entries, primary);

    // Category distribution
    let _ = write!(out, "CATEGORIES:");
    if !cls.structural.is_empty() {
        let _ = write!(out, " STRUCTURAL {}", cls.structural.len());
    }
    let mut first = cls.structural.is_empty();
    for (cat, indices) in &cls.categories {
        let sep = if first { "" } else { " |" };
        first = false;
        let _ = write!(out, "{} {} {}", sep, cat, indices.len());
    }
    for (tag, indices) in &cls.dynamic {
        let _ = write!(out, " | {} {}", tag.to_uppercase(), indices.len());
    }
    if !cls.untagged.is_empty() {
        let _ = write!(out, " | UNTAGGED {}", cls.untagged.len());
    }
    let _ = writeln!(out, "\n");

    // Hot: top 5 by relevance
    let mut hot: Vec<usize> = (0..entries.len()).collect();
    hot.sort_by(|&a, &b|
        entries[b].relevance.partial_cmp(&entries[a].relevance)
            .unwrap_or(std::cmp::Ordering::Equal));
    let _ = writeln!(out, "HOT:");
    for &i in hot.iter().take(5) {
        format_oneliner(&mut out, &entries[i]);
    }

    // Gaps
    write_gaps(&mut out, entries, primary);

    // Stats + hint
    let pct = if raw_count > 0 { 100 - (entries.len() * 100 / raw_count) } else { 0 };
    let _ = writeln!(out, "\nSTATS: {} compressed ({}% reduction) | detail='scan' for categories, 'full' for everything",
        entries.len(), pct);
    out
}

// --- Tier 2: Scan (~50 lines) ---

fn format_scan(entries: &[Compressed], query: &str, raw_count: usize,
               primary: &[String], since: Option<u64>) -> String {
    let cls = classify(entries);
    let n_topics = entries.iter().map(|e| e.topic.as_str())
        .collect::<BTreeSet<_>>().len();
    let mut out = String::new();

    let since_note = since.map(|h| format!(" (since {}h)", h)).unwrap_or_default();
    let _ = writeln!(out, "=== {}{} === {} entries \u{2192} {} compressed, {} topics\n",
        query.to_uppercase(), since_note, raw_count, entries.len(), n_topics);
    write_topics_brief(&mut out, entries, primary);

    // Structural
    if !cls.structural.is_empty() {
        let _ = writeln!(out, "--- STRUCTURAL ({}) ---", cls.structural.len());
        for &i in cls.structural.iter().take(5) { format_oneliner(&mut out, &entries[i]); }
        if cls.structural.len() > 5 {
            let _ = writeln!(out, "  ... +{} more", cls.structural.len() - 5);
        }
        let _ = writeln!(out);
    }

    // Categories: top 3 oneliners each
    for (cat, indices) in &cls.categories {
        let _ = writeln!(out, "--- {} ({}) ---", cat, indices.len());
        for &i in indices.iter().take(3) { format_oneliner(&mut out, &entries[i]); }
        if indices.len() > 3 {
            let _ = writeln!(out, "  ... +{} more", indices.len() - 3);
        }
        let _ = writeln!(out);
    }

    // Dynamic
    for (tag, indices) in &cls.dynamic {
        let _ = writeln!(out, "--- {} ({}) ---", tag.to_uppercase(), indices.len());
        for &i in indices.iter().take(3) { format_oneliner(&mut out, &entries[i]); }
        if indices.len() > 3 {
            let _ = writeln!(out, "  ... +{} more", indices.len() - 3);
        }
        let _ = writeln!(out);
    }

    // Untagged
    if !cls.untagged.is_empty() {
        let _ = writeln!(out, "--- UNTAGGED ({}) ---", cls.untagged.len());
        for &i in cls.untagged.iter().take(3) { format_oneliner(&mut out, &entries[i]); }
        if cls.untagged.len() > 3 {
            let _ = writeln!(out, "  ... +{} more", cls.untagged.len() - 3);
        }
        let _ = writeln!(out);
    }

    write_stats(&mut out, entries, raw_count);
    out
}

// --- Tier 3: Full (current behavior) ---

fn format_full(entries: &[Compressed], query: &str, raw_count: usize,
               primary: &[String], since: Option<u64>) -> String {
    let cls = classify(entries);
    let n_topics = entries.iter().map(|e| e.topic.as_str())
        .collect::<BTreeSet<_>>().len();
    let mut out = String::new();

    let since_note = since.map(|h| format!(" (since {}h)", h)).unwrap_or_default();
    let _ = writeln!(out, "=== {}{} === {} entries \u{2192} {} compressed, {} topics\n",
        query.to_uppercase(), since_note, raw_count, entries.len(), n_topics);
    write_topics(&mut out, entries, primary);
    write_graph(&mut out, entries, primary);

    // Structural
    if !cls.structural.is_empty() {
        let _ = writeln!(out, "--- STRUCTURAL ({}) ---", cls.structural.len());
        for &i in cls.structural.iter().take(5) {
            let e = &entries[i];
            let summary = e.body.lines()
                .find(|l| l.starts_with("## Summary") || l.starts_with("## "))
                .or_else(|| e.body.lines().nth(1))
                .unwrap_or("");
            let _ = writeln!(out, "  [{}] {}{}",
                e.topic,
                crate::text::truncate(summary.trim_start_matches("## ")
                    .trim_start_matches("Summary").trim(), 100),
                freshness_tag(e.days_old));
        }
        for &i in cls.structural.iter().skip(5).take(5) {
            format_oneliner(&mut out, &entries[i]);
        }
        if cls.structural.len() > 10 {
            let _ = writeln!(out, "  ... +{} more structural entries", cls.structural.len() - 10);
        }
        let _ = writeln!(out);
    }

    // Categories with full entries
    for (cat, indices) in &cls.categories {
        let _ = writeln!(out, "--- {} ({}) ---", cat, indices.len());
        let body_limit = if *cat == "DATA FLOW" { 10 } else { 5 };
        for &i in indices.iter().take(5) { format_entry_n(&mut out, &entries[i], body_limit); }
        let rest = indices.len().saturating_sub(5);
        let oneliners = rest.min(10);
        for &i in indices.iter().skip(5).take(oneliners) {
            format_oneliner(&mut out, &entries[i]);
        }
        if rest > oneliners {
            let _ = writeln!(out, "  ... +{} more {} entries\n", rest - oneliners,
                cat.to_lowercase());
        }
    }

    // Dynamic categories
    for (tag, indices) in &cls.dynamic {
        let _ = writeln!(out, "--- {} ({}) ---", tag.to_uppercase(), indices.len());
        for &i in indices.iter().take(3) { format_entry_n(&mut out, &entries[i], 5); }
        for &i in indices.iter().skip(3).take(5) { format_oneliner(&mut out, &entries[i]); }
        if indices.len() > 8 {
            let _ = writeln!(out, "  ... +{} more\n", indices.len() - 8);
        }
    }

    // Untagged: group by topic, budget primary=5, other=2
    if !cls.untagged.is_empty() {
        let _ = writeln!(out, "--- UNTAGGED ({}) ---", cls.untagged.len());
        let mut by_topic: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &i in &cls.untagged {
            by_topic.entry(entries[i].topic.as_str()).or_default().push(i);
        }
        let mut shown = 0usize;
        let mut hidden = 0usize;
        for (topic, group) in &by_topic {
            let budget = if primary.iter().any(|p| p == topic) { 5 } else { 2 };
            for &i in group.iter().take(budget) {
                format_oneliner(&mut out, &entries[i]);
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

    write_gaps(&mut out, entries, primary);
    write_stats(&mut out, entries, raw_count);
    out
}

// --- Shared helpers ---

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

fn write_topics_brief(out: &mut String, entries: &[Compressed], primary: &[String]) {
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
    let mut by_topic: BTreeMap<&str, Vec<&Compressed>> = BTreeMap::new();
    for e in entries {
        if primary.iter().any(|p| p == &e.topic) {
            by_topic.entry(&e.topic).or_default().push(e);
        }
    }
    let mut topic_tags: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for e in entries {
        if !primary.iter().any(|p| p == &e.topic) { continue; }
        let counts = topic_tags.entry(&e.topic).or_default();
        for t in &e.tags { *counts.entry(t.as_str()).or_default() += 1; }
    }
    let mut edges: Vec<(&str, &str, usize, String)> = Vec::new();
    for src in primary {
        let src_entries = match by_topic.get(src.as_str()) {
            Some(v) => v,
            None => continue,
        };
        for tgt in primary {
            if src == tgt { continue; }
            let refs: usize = src_entries.iter().map(|e| count_ci(&e.body, tgt)).sum();
            if refs == 0 { continue; }
            let edge_type = topic_tags.get(src.as_str())
                .and_then(|st| topic_tags.get(tgt.as_str()).map(|tt| (st, tt)))
                .and_then(|(st, tt)| {
                    st.keys().filter(|k| tt.contains_key(*k))
                        .max_by_key(|k| {
                            let boost = if CORE_TAGS.contains(k) { 100 } else { 0 };
                            st.get(*k).unwrap_or(&0) + tt.get(*k).unwrap_or(&0) + boost
                        })
                        .map(|k| k.to_string())
                })
                .unwrap_or_default();
            edges.push((src, tgt, refs, edge_type));
        }
    }
    edges.sort_by(|a, b| b.2.cmp(&a.2));
    if !edges.is_empty() {
        let _ = write!(out, "GRAPH:");
        for (s, t, n, etype) in edges.iter().take(6) {
            if etype.is_empty() {
                let _ = write!(out, " {} \u{2192} {} ({})", s, t, n);
            } else {
                let _ = write!(out, " {} \u{2192}[{}] {} ({})", s, etype, t, n);
            }
        }
        let _ = writeln!(out, "\n");
    }
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
        let _ = writeln!(out, "\nGAPS ({} missing core tags):", suggestions.len());
        for s in &suggestions { let _ = writeln!(out, "{}", s); }
    }
}

fn write_stats(out: &mut String, entries: &[Compressed], raw_count: usize) {
    let tagged = entries.iter().filter(|e| !e.tags.is_empty()).count();
    let sourced = entries.iter().filter(|e| e.source.is_some()).count();
    let chained = entries.iter().filter(|e| e.chain.is_some()).count();
    let pct = if raw_count > 0 { 100 - (entries.len() * 100 / raw_count) } else { 0 };
    let _ = writeln!(out, "\nSTATS: {} entries, {} tagged, {} sourced, {} chained | compressed {}\u{2192}{} ({}% reduction)",
        entries.len(), tagged, sourced, chained, raw_count, entries.len(), pct);
}

fn format_entry_n(out: &mut String, e: &Compressed, max_lines: usize) {
    let src = e.source.as_deref().map(|s| format!(" \u{2192} {s}")).unwrap_or_default();
    let also = format_also(&e.also_in);
    let chain_note = match &e.chain {
        Some(c) if c.starts_with("superseded") => " [SUPERSEDED]",
        Some(_) => " (chained)",
        None => "",
    };
    let refs = if e.link_in >= 2 { format!(" ({} refs)", e.link_in) } else { String::new() };
    let _ = writeln!(out, "[{}] {}{}{}{}{}{}", e.topic, e.date, freshness_tag(e.days_old),
        src, also, chain_note, refs);
    if let Some(ref chain) = e.chain {
        let _ = writeln!(out, "  {}", crate::text::truncate(chain, 120));
    }
    let lines: Vec<&str> = e.body.lines()
        .filter(|l| !crate::text::is_metadata_line(l))
        .collect();
    for l in lines.iter().take(max_lines) { let _ = writeln!(out, "  {}", l.trim()); }
    if lines.len() > max_lines {
        let _ = writeln!(out, "  ...({} more lines)", lines.len() - max_lines);
    }
    let _ = writeln!(out);
}

fn format_oneliner(out: &mut String, e: &Compressed) {
    let fc = crate::text::truncate(first_content(&e.body), 80);
    let src = e.source.as_deref().map(|s| format!(" \u{2192} {s}")).unwrap_or_default();
    let also = format_also(&e.also_in);
    let chain = match &e.chain {
        Some(c) if c.starts_with("superseded") => " [SUPERSEDED]".to_string(),
        Some(c) => format!(" ({})", crate::text::truncate(c, 40)),
        None => String::new(),
    };
    let refs = if e.link_in >= 2 { format!(" ({} refs)", e.link_in) } else { String::new() };
    let _ = writeln!(out, "  [{}] {}{}{}{}{}{}", e.topic, fc, src, also, chain,
        freshness_tag(e.days_old), refs);
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
fn count_ci(haystack: &str, needle: &str) -> usize {
    let nb = needle.as_bytes();
    if nb.is_empty() || nb.len() > haystack.len() { return 0; }
    haystack.as_bytes().windows(nb.len())
        .filter(|w| w.iter().zip(nb).all(|(h, n)| h.to_ascii_lowercase() == *n))
        .count()
}
