//! BM25 scoring engine. Pure scoring + corpus loading, no formatting.
//! Index-accelerated path with cache-backed corpus fallback.
//! Tag-filtered queries stay on index path when tag is in top-32 bitmap.

use std::path::Path;
pub const BM25_K1: f64 = 1.2;
pub const BM25_B: f64 = 0.75;

/// A prepared section ready for BM25 scoring.
pub struct PrepSection {
    pub name: String,
    pub lines: Vec<String>,
    pub tokens: Vec<String>,
    pub word_count: usize,
    pub tags_raw: Option<String>,
}

/// A scored search result.
pub struct ScoredResult {
    pub name: String,
    pub lines: Vec<String>,
    pub score: f64,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SearchMode { And, Or }

/// Filter options for search (date range + tag + topic scope + mode).
pub struct Filter {
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub tag: Option<String>,
    pub topic: Option<String>,
    pub mode: SearchMode,
}

impl Filter {
    pub fn none() -> Self {
        Self { after: None, before: None, tag: None, topic: None, mode: SearchMode::And }
    }
    pub fn is_active(&self) -> bool {
        self.after.is_some() || self.before.is_some() || self.tag.is_some() || self.topic.is_some()
    }
}

/// Load corpus from cache (mtime-invalidated), applying filters.
pub fn load_corpus(dir: &Path, filter: &Filter) -> Result<Vec<PrepSection>, String> {
    crate::cache::with_corpus(dir, |cached| {
        let mut corpus = Vec::new();
        for e in cached {
            if let Some(ref t) = filter.topic { if e.topic != *t { continue; } }
            if !passes_filter_cached(e, filter) { continue; }
            let date = crate::time::minutes_to_date_str(e.timestamp_min);
            let mut lines = vec![format!("## {date}")];
            let mut tags_raw = None;
            for line in e.body.lines() {
                if line.starts_with("[tags: ") { tags_raw = Some(line.to_string()); }
                lines.push(line.to_string());
            }
            corpus.push(PrepSection {
                name: e.topic.clone(), lines,
                tokens: e.tokens.clone(), word_count: e.word_count,
                tags_raw,
            });
        }
        corpus
    })
}

/// Score corpus against query terms. Returns (results, was_fallback).
pub fn score_corpus(corpus: &[PrepSection], terms: &[String], mode: SearchMode)
    -> (Vec<ScoredResult>, bool)
{
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
    if !terms.is_empty() {
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    }
    (results, fallback)
}

/// Check if tokens match query terms in given mode.
pub fn matches_tokens(tokens: &[String], terms: &[String], mode: SearchMode) -> bool {
    if terms.is_empty() { return true; }
    match mode {
        SearchMode::And => terms.iter().all(|t| tokens.contains(t)),
        SearchMode::Or => terms.iter().any(|t| tokens.contains(t)),
    }
}

fn score_mode(corpus: &[PrepSection], terms: &[String], mode: SearchMode,
              n: f64, avgdl: f64, dfs: &[usize]) -> Vec<ScoredResult> {
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
        let topic_lower = ps.name.to_lowercase();
        if terms.iter().any(|t| topic_lower.contains(t.as_str())) { score *= 1.5; }
        if let Some(ref tag_line) = ps.tags_raw {
            let tag_lower = tag_line.to_lowercase();
            let tag_hits = terms.iter().filter(|t| tag_lower.contains(t.as_str())).count();
            if tag_hits > 0 { score *= 1.0 + 0.3 * tag_hits as f64; }
        }
        Some(ScoredResult { name: ps.name.clone(), lines: ps.lines.clone(), score })
    }).collect()
}

fn passes_filter_cached(e: &crate::cache::CachedEntry, f: &Filter) -> bool {
    if f.after.is_some() || f.before.is_some() {
        let days = e.timestamp_min as i64 / 1440;
        if let Some(after) = f.after { if days < after { return false; } }
        if let Some(before) = f.before { if days > before { return false; } }
    }
    if let Some(ref tag) = f.tag {
        let tl = tag.to_lowercase();
        let has = e.tags_raw.as_ref().map(|line|
            line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']'))
                .map(|inner| inner.split(',').any(|t| t.trim().to_lowercase() == tl))
                .unwrap_or(false)
        ).unwrap_or(false);
        if !has { return false; }
    }
    true
}

/// Unified search: tries binary index first, falls back to cached corpus scan.
/// Tag-filtered queries use index path when tag is in top-32 bitmap.
pub fn search_scored(dir: &Path, terms: &[String], filter: &Filter, limit: Option<usize>)
    -> Result<(Vec<ScoredResult>, bool), String>
{
    if terms.is_empty() {
        let corpus = load_corpus(dir, filter)?;
        return Ok(score_corpus(&corpus, terms, filter.mode));
    }

    // Try index path — even tag filters work if tag is in top-32 bitmap
    let index_path = dir.join("index.bin");
    if let Ok(data) = std::fs::read(&index_path) {
        // Check if tag filter can be served by index
        let tag_on_index = match &filter.tag {
            None => true,
            Some(tag) => crate::binquery::resolve_tag(&data, tag).is_some(),
        };
        if tag_on_index {
            if let Ok(result) = score_via_index(dir, &data, terms, filter, limit) {
                return Ok(result);
            }
        }
    }

    // Fallback: cached corpus scan
    let corpus = load_corpus(dir, filter)?;
    Ok(score_corpus(&corpus, terms, filter.mode))
}

/// Score using binary inverted index with FilterPred for pre-scoring elimination.
fn score_via_index(dir: &Path, index_data: &[u8], terms: &[String],
                   filter: &Filter, limit: Option<usize>)
    -> Result<(Vec<ScoredResult>, bool), String>
{
    // Build FilterPred from Filter
    let topic_id = match &filter.topic {
        Some(name) => crate::binquery::resolve_topic(index_data, name),
        None => None,
    };
    let after_days = filter.after.map(|d| d.max(0) as u16).unwrap_or(0);
    let before_days = filter.before.map(|d| d.min(u16::MAX as i64) as u16).unwrap_or(u16::MAX);
    let tag_mask = match &filter.tag {
        Some(tag) => crate::binquery::resolve_tag(index_data, tag)
            .map(|bit| 1u32 << bit).unwrap_or(0),
        None => 0,
    };

    let pred = crate::binquery::FilterPred { topic_id, after_days, before_days, tag_mask };
    let index_limit = limit.unwrap_or(200).max(100);
    let hits = crate::binquery::search_v2_filtered(index_data, &terms.join(" "), &pred, index_limit)?;

    if hits.is_empty() {
        if filter.mode == SearchMode::And && terms.len() >= 2 {
            let corpus = load_corpus(dir, filter)?;
            let n = corpus.len() as f64;
            let avgdl = if corpus.is_empty() { 1.0 } else {
                corpus.iter().map(|s| s.word_count).sum::<usize>() as f64 / n
            };
            let dfs: Vec<usize> = terms.iter()
                .map(|t| corpus.iter().filter(|s| s.tokens.contains(t)).count()).collect();
            let results = score_mode(&corpus, terms, SearchMode::Or, n, avgdl, &dfs);
            if !results.is_empty() { return Ok((results, true)); }
        }
        return Ok((Vec::new(), false));
    }

    // Fetch full entry bodies from data.log only for displayed results
    let topics = crate::binquery::topic_table(index_data)?;
    let topic_names: std::collections::HashMap<u16, String> = topics.iter()
        .map(|(id, name, _)| (*id, name.clone())).collect();
    let log_path = crate::config::log_path(dir);
    let mut results = Vec::new();

    for hit in &hits {
        let topic_name = match topic_names.get(&hit.topic_id) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut score = hit.score;

        // Topic-name boost
        let topic_lower = topic_name.to_lowercase();
        if terms.iter().any(|t| topic_lower.contains(t.as_str())) { score *= 1.5; }

        // Fetch full entry body for display + tag boost
        let entry = crate::datalog::read_entry(&log_path, hit.log_offset)
            .unwrap_or(crate::datalog::LogEntry {
                offset: hit.log_offset, topic: topic_name.clone(),
                body: String::new(), timestamp_min: hit.date_minutes,
            });

        // Tag boost
        for line in entry.body.lines() {
            if line.starts_with("[tags: ") {
                let tag_lower = line.to_lowercase();
                let tag_hits = terms.iter().filter(|t| tag_lower.contains(t.as_str())).count();
                if tag_hits > 0 { score *= 1.0 + 0.3 * tag_hits as f64; }
                break;
            }
        }

        let date = crate::time::minutes_to_date_str(entry.timestamp_min);
        let mut lines = vec![format!("## {date}")];
        for line in entry.body.lines() { lines.push(line.to_string()); }
        results.push(ScoredResult { name: topic_name, lines, score });
    }
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok((results, false))
}

/// Collect all tags from cache for no-match suggestions.
pub fn collect_all_tags(dir: &Path) -> Vec<(String, usize)> {
    crate::cache::with_corpus(dir, |cached| {
        let mut tags: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for e in cached {
            if let Some(ref line) = e.tags_raw {
                if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                    for tag in inner.split(',') {
                        let t = tag.trim().to_lowercase();
                        if !t.is_empty() { *tags.entry(t).or_insert(0) += 1; }
                    }
                }
            }
        }
        let mut sorted: Vec<(String, usize)> = tags.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted
    }).unwrap_or_default()
}
