use std::fmt::Write;
use std::path::Path;
use crate::text::{query_terms, tokenize, truncate, extract_tags};

/// Filter options for search (date range + tag + topic scope + mode).
pub struct Filter {
    pub after: Option<i64>,  // days since epoch
    pub before: Option<i64>,
    pub tag: Option<String>,
    pub topic: Option<String>,
    pub mode: SearchMode,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SearchMode { And, Or }

impl Filter {
    pub fn none() -> Self { Self { after: None, before: None, tag: None, topic: None, mode: SearchMode::And } }
    pub fn is_active(&self) -> bool { self.after.is_some() || self.before.is_some() || self.tag.is_some() || self.topic.is_some() }
}

pub fn run(dir: &Path, query: &str, plain: bool, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    let terms = query_terms(query);
    if terms.is_empty() && !filter.is_active() { return Err("provide a query or filter".into()); }
    let corpus = load_corpus(dir, filter)?;
    let (results, fallback) = score_corpus(&corpus, &terms, filter.mode);
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
    let corpus = load_corpus(dir, filter)?;
    let (results, fallback) = score_corpus(&corpus, &terms, filter.mode);
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
    let corpus = load_corpus(dir, filter)?;
    let (results, fallback) = score_corpus(&corpus, &terms, filter.mode);
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
    let corpus = load_corpus(dir, filter)?;
    let count_fn = |mode: SearchMode| -> Vec<(String, usize)> {
        let mut hits: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for ps in &corpus { if matches_tokens(&ps.tokens, &terms, mode) { *hits.entry(ps.name.clone()).or_insert(0) += 1; } }
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
    let corpus = load_corpus(dir, filter)?;
    let do_count = |mode: SearchMode| -> (usize, usize) {
        let mut total = 0; let mut topics = std::collections::HashSet::new();
        for ps in &corpus { if matches_tokens(&ps.tokens, &terms, mode) { total += 1; topics.insert(ps.name.clone()); } }
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
    let corpus = load_corpus(dir, filter)?;
    let (results, fallback) = score_corpus(&corpus, &terms, filter.mode);
    if results.is_empty() { return Ok(no_match_message(query, filter, dir)); }
    let cap = limit_per_topic.unwrap_or(5);
    let mut groups: std::collections::BTreeMap<String, Vec<&ScoredResult>> = std::collections::BTreeMap::new();
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

// --- Core BM25 ---
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

struct PrepSection { name: String, lines: Vec<String>, tokens: Vec<String>, word_count: usize }
struct ScoredResult { name: String, lines: Vec<String>, score: f64 }

fn load_corpus(dir: &Path, filter: &Filter) -> Result<Vec<PrepSection>, String> {
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Err("no data.log found".into()); }
    let entries = crate::datalog::iter_live(&log_path)?;
    let mut corpus = Vec::new();
    for e in &entries {
        if let Some(ref t) = filter.topic { if e.topic != *t { continue; } }
        if !passes_filter_entry(e, filter) { continue; }
        let date = crate::time::minutes_to_date_str(e.timestamp_min);
        let mut lines = vec![format!("## {date}")];
        for line in e.body.lines() { lines.push(line.to_string()); }
        let tokens = tokenize(&e.body);
        let word_count = tokens.len();
        corpus.push(PrepSection { name: e.topic.clone(), lines, tokens, word_count });
    }
    Ok(corpus)
}

fn passes_filter_entry(e: &crate::datalog::LogEntry, f: &Filter) -> bool {
    if f.after.is_some() || f.before.is_some() {
        let days = e.timestamp_min as i64 / 1440;
        if let Some(after) = f.after { if days < after { return false; } }
        if let Some(before) = f.before { if days > before { return false; } }
    }
    if let Some(ref tag) = f.tag {
        let tl = tag.to_lowercase();
        let has = e.body.lines().any(|line|
            line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']'))
                .map(|inner| inner.split(',').any(|t| t.trim().to_lowercase() == tl))
                .unwrap_or(false));
        if !has { return false; }
    }
    true
}

fn score_corpus(corpus: &[PrepSection], terms: &[String], mode: SearchMode) -> (Vec<ScoredResult>, bool) {
    let n = corpus.len() as f64;
    let total_words: usize = corpus.iter().map(|s| s.word_count).sum();
    let avgdl = if corpus.is_empty() { 1.0 } else { total_words as f64 / n };
    let dfs: Vec<usize> = terms.iter()
        .map(|t| corpus.iter().filter(|s| s.tokens.contains(t)).count()).collect();
    let mut results = score_mode(corpus, terms, mode, n, avgdl, &dfs);
    let mut fallback = false;
    if results.is_empty() && mode == SearchMode::And && terms.len() >= 2 {
        results = score_mode(corpus, terms, SearchMode::Or, n, avgdl, &dfs);
        fallback = !results.is_empty();
    }
    if !terms.is_empty() { results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)); }
    (results, fallback)
}

fn score_mode(corpus: &[PrepSection], terms: &[String], mode: SearchMode, n: f64, avgdl: f64, dfs: &[usize]) -> Vec<ScoredResult> {
    corpus.iter().filter(|ps| matches_tokens(&ps.tokens, terms, mode)).filter_map(|ps| {
        let len_norm = 1.0 - BM25_B + BM25_B * ps.word_count as f64 / avgdl.max(1.0);
        let mut score = 0.0;
        for (i, term) in terms.iter().enumerate() {
            let tf = ps.tokens.iter().filter(|t| *t == term).count() as f64;
            if tf == 0.0 { continue; }
            let df = dfs[i] as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            score += idf * (tf * (BM25_K1 + 1.0)) / (tf + BM25_K1 * len_norm);
        }
        if score == 0.0 { return None; }
        // Topic-name boost: entries in a topic matching query terms rank higher
        let topic_lower = ps.name.to_lowercase();
        if terms.iter().any(|t| topic_lower.contains(t.as_str())) { score *= 1.5; }
        // Tag boost: curated tags matching query terms are high-signal
        if let Some(tag_line) = ps.lines.iter().find(|l| l.starts_with("[tags: ")) {
            let tag_lower = tag_line.to_lowercase();
            let tag_hits = terms.iter().filter(|t| tag_lower.contains(t.as_str())).count();
            if tag_hits > 0 { score *= 1.0 + 0.3 * tag_hits as f64; }
        }
        Some(ScoredResult { name: ps.name.clone(), lines: ps.lines.clone(), score })
    }).collect()
}

fn matches_tokens(tokens: &[String], terms: &[String], mode: SearchMode) -> bool {
    if terms.is_empty() { return true; }
    match mode { SearchMode::And => terms.iter().all(|t| tokens.contains(t)),
                  SearchMode::Or => terms.iter().any(|t| tokens.contains(t)) }
}

fn no_match_message(query: &str, filter: &Filter, dir: &Path) -> String {
    let mut msg = String::new();
    if let Some(ref tag) = filter.tag {
        let existing = collect_all_tags(dir);
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

fn collect_all_tags(dir: &Path) -> Vec<(String, usize)> {
    let log_path = crate::config::log_path(dir);
    let entries = match crate::datalog::iter_live(&log_path) { Ok(e) => e, Err(_) => return Vec::new() };
    let mut tags: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for e in &entries {
        for line in e.body.lines() {
            if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                for tag in inner.split(',') { let t = tag.trim().to_lowercase();
                    if !t.is_empty() { *tags.entry(t).or_insert(0) += 1; } }
            }
        }
    }
    let mut sorted: Vec<(String, usize)> = tags.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted
}
