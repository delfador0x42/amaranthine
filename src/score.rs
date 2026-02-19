//! BM25 scoring engine. Pure scoring + corpus loading, no formatting.
//! Index-accelerated path with cache-backed corpus fallback.
//! Tag-filtered queries stay on index path when tag is in top-32 bitmap.

use std::collections::{HashSet, HashMap};
use std::path::Path;
use std::rc::Rc;
pub const BM25_K1: f64 = 1.2;
pub const BM25_B: f64 = 0.75;

/// A prepared section ready for BM25 scoring.
pub struct PrepSection {
    pub name: String,
    pub lines: Rc<Vec<String>>,
    pub token_set: HashSet<String>,
    pub tf_map: HashMap<String, usize>,
    pub word_count: usize,
    pub tags_raw: Option<String>,
}

/// A scored search result.
pub struct ScoredResult {
    pub name: String,
    pub lines: Rc<Vec<String>>,
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
                name: e.topic.clone(), lines: Rc::new(lines),
                token_set: e.token_set.clone(), tf_map: e.tf_map.clone(),
                word_count: e.word_count, tags_raw,
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
        .map(|t| corpus.iter().filter(|s| s.token_set.contains(t)).count()).collect();
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

/// Check if tokens match query terms in given mode. O(terms) via HashSet.
pub fn matches_tokens(token_set: &HashSet<String>, terms: &[String], mode: SearchMode) -> bool {
    if terms.is_empty() { return true; }
    match mode {
        SearchMode::And => terms.iter().all(|t| token_set.contains(t)),
        SearchMode::Or => terms.iter().any(|t| token_set.contains(t)),
    }
}

fn score_mode(corpus: &[PrepSection], terms: &[String], mode: SearchMode,
              n: f64, avgdl: f64, dfs: &[usize]) -> Vec<ScoredResult> {
    corpus.iter().filter(|ps| matches_tokens(&ps.token_set, terms, mode)).filter_map(|ps| {
        let len_norm = 1.0 - BM25_B + BM25_B * ps.word_count as f64 / avgdl.max(1.0);
        let mut score = 0.0;
        for (i, term) in terms.iter().enumerate() {
            let tf = *ps.tf_map.get(term).unwrap_or(&0) as f64;
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
/// F2: Accepts pre-cached index data to avoid redundant disk reads.
pub fn search_scored(dir: &Path, terms: &[String], filter: &Filter, limit: Option<usize>,
                     index_data: Option<&[u8]>)
    -> Result<(Vec<ScoredResult>, bool), String>
{
    if terms.is_empty() {
        let corpus = load_corpus(dir, filter)?;
        return Ok(score_corpus(&corpus, terms, filter.mode));
    }

    // Try index path — prefer cached data, fall back to disk read
    let fallback_data;
    let data = match index_data {
        Some(d) => Some(d),
        None => {
            fallback_data = std::fs::read(dir.join("index.bin")).ok();
            fallback_data.as_deref()
        }
    };
    if let Some(data) = data {
        let tag_on_index = match &filter.tag {
            None => true,
            Some(tag) => crate::binquery::resolve_tag(data, tag).is_some(),
        };
        if tag_on_index {
            if let Ok(result) = score_via_index(dir, data, terms, filter, limit) {
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
    let pred = build_filter_pred(index_data, filter);
    let index_limit = limit.unwrap_or(200).max(100);
    let query_str = terms.join(" ");
    let hits = crate::binquery::search_v2_filtered(index_data, &query_str, &pred, index_limit)?;

    if hits.is_empty() && filter.mode == SearchMode::And && terms.len() >= 2 {
        // F-OR: Try OR on index before expensive corpus fallback (5.35ms → ~10µs)
        let or_hits = crate::binquery::search_v2_or(index_data, &query_str, &pred, index_limit)?;
        if !or_hits.is_empty() {
            return hydrate_index_hits(dir, index_data, terms, &or_hits, true);
        }
        return Ok((Vec::new(), false));
    }

    hydrate_index_hits(dir, index_data, terms, &hits, false)
}

fn build_filter_pred(index_data: &[u8], filter: &Filter) -> crate::binquery::FilterPred {
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
    crate::binquery::FilterPred { topic_id, after_days, before_days, tag_mask }
}

/// Hydrate index hits into ScoredResults with full entry bodies from data.log.
fn hydrate_index_hits(dir: &Path, index_data: &[u8], terms: &[String],
                      hits: &[crate::binquery::SearchHit], fallback: bool)
    -> Result<(Vec<ScoredResult>, bool), String>
{
    if hits.is_empty() { return Ok((Vec::new(), false)); }

    // F10: Lazy topic name cache — only resolve topic_ids we actually hit
    let mut name_cache: HashMap<u16, String> = HashMap::new();
    let log_path = crate::config::log_path(dir);
    let mut log_file = std::fs::File::open(&log_path)
        .map_err(|e| format!("open data.log: {e}"))?;
    let mut results = Vec::new();

    for hit in hits {
        let topic_name = match name_cache.get(&hit.topic_id) {
            Some(n) => n.clone(),
            None => match crate::binquery::topic_name(index_data, hit.topic_id) {
                Ok(n) => { name_cache.insert(hit.topic_id, n.clone()); n }
                Err(_) => continue,
            },
        };
        let mut score = hit.score;

        // Topic-name boost
        let topic_lower = topic_name.to_lowercase();
        if terms.iter().any(|t| topic_lower.contains(t.as_str())) { score *= 1.5; }

        // Fetch full entry body for display + tag boost (single file handle)
        let entry = crate::datalog::read_entry_from(&mut log_file, hit.log_offset)
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
        results.push(ScoredResult { name: topic_name, lines: Rc::new(lines), score });
    }
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok((results, fallback))
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
