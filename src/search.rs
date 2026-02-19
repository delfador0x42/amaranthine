//! Search formatting: takes scored results and produces output.
//! Scoring lives in score.rs (index-accelerated with corpus fallback).

use std::fmt::Write;
use std::path::Path;
use crate::text::{query_terms, truncate, extract_tags};
pub use crate::score::{Filter, SearchMode};

pub fn run(dir: &Path, query: &str, plain: bool, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    let terms = query_terms(query);
    if terms.is_empty() && !filter.is_active() { return Err("provide a query or filter".into()); }
    let (results, fallback) = crate::score::search_scored(dir, &terms, filter, limit)?;
    let total = results.len();
    let show = limit.map(|l| total.min(l)).unwrap_or(total);

    let mut out = String::new();
    if fallback { let _ = writeln!(out, "(no exact match — showing {} OR results)", results.len()); }
    let mut last_file = String::new();
    for r in results.iter().take(show) {
        if r.name != last_file {
            if plain { let _ = writeln!(out, "\n--- {} ---", r.name); }
            else { let _ = writeln!(out, "\n\x1b[1;36m--- {} ---\x1b[0m", r.name); }
            last_file = r.name.clone();
        }
        for line in &r.lines {
            if !terms.is_empty() && terms.iter().any(|t| line.to_lowercase().contains(t.as_str())) {
                if plain { let _ = writeln!(out, "> {line}"); }
                else { let _ = writeln!(out, "\x1b[1;33m{line}\x1b[0m"); }
            } else { let _ = writeln!(out, "{line}"); }
        }
        let _ = writeln!(out);
    }
    if total == 0 { out.push_str(&no_match_message(query, filter, dir)); }
    else if show < total { let _ = writeln!(out, "(showing {show} of {total} matches)"); }
    else { let _ = writeln!(out, "{total} matching section(s)"); }
    Ok(out)
}

pub fn run_brief(dir: &Path, query: &str, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    let terms = query_terms(query);
    if terms.is_empty() && !filter.is_active() { return Err("provide a query or filter".into()); }
    let (results, fallback) = crate::score::search_scored(dir, &terms, filter, limit)?;
    let total = results.len();
    let show = limit.map(|l| total.min(l)).unwrap_or(total);
    let mut out = String::new();
    if fallback { let _ = writeln!(out, "(no exact match — showing OR results)"); }
    for r in results.iter().take(show) {
        let tags = extract_tags(&r.lines);
        let tag_suffix = tags.map(|t| format!(" {t}")).unwrap_or_default();
        let content = r.lines.iter().skip(1)
            .find(|l| !l.starts_with("[tags:") && !l.trim().is_empty())
            .map(|l| truncate(l.trim().trim_start_matches("- "), 80))
            .unwrap_or("");
        let _ = writeln!(out, "  [{}] {content}{tag_suffix}", r.name);
    }
    if total == 0 { out.push_str(&no_match_message(query, filter, dir)); }
    else { let _ = writeln!(out, "{total} match(es)"); }
    Ok(out)
}

pub fn run_medium(dir: &Path, query: &str, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    let terms = query_terms(query);
    if terms.is_empty() && !filter.is_active() { return Err("provide a query or filter".into()); }
    let (results, fallback) = crate::score::search_scored(dir, &terms, filter, limit)?;
    let total = results.len();
    let show = limit.map(|l| total.min(l)).unwrap_or(total);
    let mut out = String::new();
    if fallback { let _ = writeln!(out, "(no exact match — showing OR results)"); }
    for r in results.iter().take(show) {
        let header = r.lines.first().map(|s| s.as_str()).unwrap_or("??");
        let tags = extract_tags(&r.lines);
        if let Some(ref t) = tags {
            let _ = writeln!(out, "  [{}] {} {}", r.name, header.trim_start_matches("## "), t);
        } else {
            let _ = writeln!(out, "  [{}] {}", r.name, header.trim_start_matches("## "));
        }
        let mut content_lines = 0;
        for line in r.lines.iter().skip(1) {
            if line.starts_with("[tags:") || line.trim().is_empty() { continue; }
            let _ = writeln!(out, "    {}", truncate(line.trim(), 100));
            content_lines += 1;
            if content_lines >= 2 { break; }
        }
    }
    if total == 0 { out.push_str(&no_match_message(query, filter, dir)); }
    else if show < total { let _ = writeln!(out, "{total} match(es), showing {show}"); }
    else { let _ = writeln!(out, "{total} match(es)"); }
    Ok(out)
}

pub fn run_topics(dir: &Path, query: &str, filter: &Filter) -> Result<String, String> {
    let terms = query_terms(query);
    if terms.is_empty() && !filter.is_active() { return Err("provide a query or filter".into()); }
    // Topics/count needs per-entry matching, use corpus scan
    let corpus = crate::score::load_corpus(dir, filter)?;
    let count_fn = |mode: SearchMode| -> Vec<(String, usize)> {
        let mut hits: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for ps in &corpus { if crate::score::matches_tokens(&ps.tokens, &terms, mode) { *hits.entry(ps.name.clone()).or_insert(0) += 1; } }
        hits.into_iter().collect()
    };
    let mut hits = count_fn(filter.mode);
    let mut fallback = false;
    if hits.is_empty() && filter.mode == SearchMode::And && terms.len() >= 2 {
        hits = count_fn(SearchMode::Or);
        fallback = !hits.is_empty();
    }
    let total: usize = hits.iter().map(|(_, n)| n).sum();
    let mut out = String::new();
    if hits.is_empty() { out.push_str(&no_match_message(query, filter, dir)); }
    else {
        if fallback { let _ = writeln!(out, "(no exact match — showing OR results)"); }
        for (topic, n) in &hits { let _ = writeln!(out, "  {topic}: {n} hit{}", if *n == 1 { "" } else { "s" }); }
        let _ = writeln!(out, "{total} match(es) across {} topic(s)", hits.len());
    }
    Ok(out)
}

pub fn count(dir: &Path, query: &str, filter: &Filter) -> Result<String, String> {
    let terms = query_terms(query);
    if terms.is_empty() && !filter.is_active() { return Err("provide a query or filter".into()); }
    // Count needs per-entry matching, use corpus scan
    let corpus = crate::score::load_corpus(dir, filter)?;
    let do_count = |mode: SearchMode| -> (usize, usize) {
        let mut total = 0; let mut topics = std::collections::HashSet::new();
        for ps in &corpus { if crate::score::matches_tokens(&ps.tokens, &terms, mode) { total += 1; topics.insert(ps.name.clone()); } }
        (total, topics.len())
    };
    let (total, topics) = do_count(filter.mode);
    if total > 0 { return Ok(format!("{total} matches across {topics} topics for '{query}'")); }
    if filter.mode == SearchMode::And && terms.len() >= 2 {
        let (total, topics) = do_count(SearchMode::Or);
        if total > 0 { return Ok(format!("(OR fallback) {total} matches across {topics} topics for '{query}'")); }
    }
    Ok(format!("0 matches for '{query}'"))
}

pub fn run_grouped(dir: &Path, query: &str, limit_per_topic: Option<usize>, filter: &Filter) -> Result<String, String> {
    let terms = query_terms(query);
    if terms.is_empty() { return Err("query required for entity search".into()); }
    let (results, fallback) = crate::score::search_scored(dir, &terms, filter, None)?;
    if results.is_empty() { return Ok(no_match_message(query, filter, dir)); }
    let cap = limit_per_topic.unwrap_or(5);
    let mut groups: std::collections::BTreeMap<String, Vec<&crate::score::ScoredResult>> = std::collections::BTreeMap::new();
    for r in &results { groups.entry(r.name.clone()).or_default().push(r); }
    let mut topic_order: Vec<(String, f64)> = groups.iter()
        .map(|(n, e)| (n.clone(), e.first().map(|e| e.score).unwrap_or(0.0))).collect();
    topic_order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let total: usize = groups.values().map(|v| v.len()).sum();
    let mut out = String::new();
    if fallback { let _ = writeln!(out, "(no exact match — showing OR results)"); }
    let _ = writeln!(out, "'{}' across {} topics ({} matches):\n", query, groups.len(), total);
    for (name, _) in &topic_order {
        let entries = &groups[name];
        let _ = writeln!(out, "[{}] {} matches", name, entries.len());
        for r in entries.iter().take(cap) {
            let header = r.lines.first().map(|s| s.as_str()).unwrap_or("??");
            let _ = write!(out, "  {} — ", header.trim_start_matches("## "));
            if let Some(line) = r.lines.iter().skip(1)
                .find(|l| !l.starts_with("[tags:") && !l.starts_with("[source:") && !l.starts_with("[type:") && !l.trim().is_empty()) {
                let _ = writeln!(out, "{}", truncate(line.trim(), 90));
            } else { let _ = writeln!(out); }
        }
        if entries.len() > cap { let _ = writeln!(out, "  ...and {} more", entries.len() - cap); }
        let _ = writeln!(out);
    }
    Ok(out)
}

fn no_match_message(query: &str, filter: &Filter, dir: &Path) -> String {
    let mut msg = String::new();
    if let Some(ref tag) = filter.tag {
        let existing = crate::score::collect_all_tags(dir);
        if !existing.iter().any(|(t, _)| t == &tag.to_lowercase()) {
            let _ = writeln!(msg, "tag '{}' not found", tag);
            let similar: Vec<&str> = existing.iter()
                .filter(|(t, _)| t.contains(&tag.to_lowercase()) || tag.to_lowercase().contains(t.as_str()))
                .map(|(t, _)| t.as_str()).take(5).collect();
            if !similar.is_empty() { let _ = writeln!(msg, "  similar: {}", similar.join(", ")); }
            else { let top: Vec<&str> = existing.iter().take(8).map(|(t, _)| t.as_str()).collect();
                if !top.is_empty() { let _ = writeln!(msg, "  existing tags: {}", top.join(", ")); } }
            return msg;
        }
        let _ = writeln!(msg, "no entries with tag '{}' match '{}'", tag, query);
    } else { let _ = writeln!(msg, "no matches for '{query}'"); }
    msg
}
