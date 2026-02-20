//! BM25 scoring engine. Index-accelerated path with cache-backed corpus fallback.
//! Scores directly on borrowed &CachedEntry — no token_set/tf_map clones.
//! Tag-filtered queries stay on index path when tag is in top-32 bitmap.

use crate::fxhash::{FxHashSet, FxHashMap};
use std::path::Path;
use std::rc::Rc;
pub const BM25_K1: f64 = 1.2;
pub const BM25_B: f64 = 0.75;

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

/// Check if tokens match query terms in given mode. O(terms) via HashMap key lookup.
#[inline]
pub fn matches_tokens(tf_map: &FxHashMap<String, usize>, terms: &[String], mode: SearchMode) -> bool {
    if terms.is_empty() { return true; }
    match mode {
        SearchMode::And => terms.iter().all(|t| tf_map.contains_key(t)),
        SearchMode::Or => terms.iter().any(|t| tf_map.contains_key(t)),
    }
}

/// BM25 score on borrowed cache entries. Two-phase: score first, extract lines for top-K only.
/// Phase 1 does zero String allocations. Phase 2 only allocates for `limit` entries.
fn score_cached_mode(entries: &[&crate::cache::CachedEntry], terms: &[String],
                     mode: SearchMode, n: f64, avgdl: f64, dfs: &[usize],
                     limit: usize)
    -> Vec<ScoredResult>
{
    // Phase 1: Score only — zero String allocations
    let mut scored: Vec<(f64, usize)> = entries.iter().enumerate()
        .filter(|(_, e)| matches_tokens(&e.tf_map, terms, mode))
        .filter_map(|(idx, e)| {
            let len_norm = 1.0 - BM25_B + BM25_B * e.word_count as f64 / avgdl.max(1.0);
            let mut score = 0.0;
            for (i, term) in terms.iter().enumerate() {
                let tf = *e.tf_map.get(term).unwrap_or(&0) as f64;
                if tf == 0.0 { continue; }
                let df = dfs[i] as f64;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                score += idf * (tf * (BM25_K1 + 1.0)) / (tf + BM25_K1 * len_norm);
            }
            if score == 0.0 { return None; }
            debug_assert!(e.topic.chars().all(|c| !c.is_uppercase()));
            if terms.iter().any(|t| e.topic.contains(t.as_str())) { score *= 1.5; }
            if let Some(ref tag_line) = e.tags_raw {
                let tag_hits = terms.iter().filter(|t| tag_line.contains(t.as_str())).count();
                if tag_hits > 0 { score *= 1.0 + 0.3 * tag_hits as f64; }
            }
            Some((score, idx))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    // Phase 2: Extract lines ONLY for top-K entries
    scored.truncate(limit);
    scored.iter().map(|&(score, idx)| {
        let e = entries[idx];
        let mut lines = vec![format!("## {}", e.date_str())];
        for line in e.body.lines() { lines.push(line.to_string()); }
        ScoredResult { name: e.topic.to_string(), lines: Rc::new(lines), score }
    }).collect()
}

/// Score on cache with AND→OR fallback. Borrows token_set/tf_map from cache.
fn score_on_cache(dir: &Path, terms: &[String], filter: &Filter, limit: Option<usize>)
    -> Result<(Vec<ScoredResult>, bool), String>
{
    crate::cache::with_corpus(dir, |cached| {
        let filtered: Vec<&crate::cache::CachedEntry> = cached.iter()
            .filter(|e| {
                if let Some(ref t) = filter.topic { if e.topic != *t { return false; } }
                passes_filter_cached(e, filter)
            })
            .collect();
        let n = filtered.len() as f64;
        let total_words: usize = filtered.iter().map(|e| e.word_count).sum();
        let avgdl = if filtered.is_empty() { 1.0 } else { total_words as f64 / n };
        let dfs: Vec<usize> = terms.iter()
            .map(|t| filtered.iter().filter(|e| e.tf_map.contains_key(t)).count()).collect();
        let cap = limit.unwrap_or(filtered.len());
        let mut results = score_cached_mode(&filtered, terms, filter.mode, n, avgdl, &dfs, cap);
        let mut fallback = false;
        if results.is_empty() && filter.mode == SearchMode::And && terms.len() >= 2 {
            results = score_cached_mode(&filtered, terms, SearchMode::Or, n, avgdl, &dfs, cap);
            fallback = !results.is_empty();
        }
        (results, fallback)
    })
}

/// Count matches per topic directly on cache. Zero clones.
pub fn topic_matches_cached(dir: &Path, terms: &[String], filter: &Filter)
    -> Result<(Vec<(String, usize)>, bool), String>
{
    crate::cache::with_corpus(dir, |cached| {
        let count_fn = |mode: SearchMode| -> Vec<(String, usize)> {
            let mut hits: FxHashMap<&str, usize> = FxHashMap::default();
            for e in cached {
                if let Some(ref t) = filter.topic { if e.topic != *t { continue; } }
                if !passes_filter_cached(e, filter) { continue; }
                if matches_tokens(&e.tf_map, terms, mode) {
                    *hits.entry(&e.topic).or_insert(0) += 1;
                }
            }
            hits.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        };
        let mut hits = count_fn(filter.mode);
        let mut fallback = false;
        if hits.is_empty() && filter.mode == SearchMode::And && terms.len() >= 2 {
            hits = count_fn(SearchMode::Or);
            fallback = !hits.is_empty();
        }
        (hits, fallback)
    })
}

/// Count total matches + unique topics on cache. Zero clones.
pub fn count_on_cache(dir: &Path, terms: &[String], filter: &Filter)
    -> Result<(usize, usize, bool), String>
{
    crate::cache::with_corpus(dir, |cached| {
        let do_count = |mode: SearchMode| -> (usize, usize) {
            let mut total = 0;
            let mut topics: FxHashSet<&str> = FxHashSet::default();
            for e in cached {
                if let Some(ref t) = filter.topic { if e.topic != *t { continue; } }
                if !passes_filter_cached(e, filter) { continue; }
                if matches_tokens(&e.tf_map, terms, mode) {
                    total += 1;
                    topics.insert(&e.topic);
                }
            }
            (total, topics.len())
        };
        let (total, topics) = do_count(filter.mode);
        if total > 0 { return (total, topics, false); }
        if filter.mode == SearchMode::And && terms.len() >= 2 {
            let (total, topics) = do_count(SearchMode::Or);
            return (total, topics, total > 0);
        }
        (0, 0, false)
    })
}

fn passes_filter_cached(e: &crate::cache::CachedEntry, f: &Filter) -> bool {
    if f.after.is_some() || f.before.is_some() {
        let days = e.day();
        if let Some(after) = f.after { if days < after { return false; } }
        if let Some(before) = f.before { if days > before { return false; } }
    }
    if let Some(ref tag) = f.tag {
        if !e.has_tag(tag) { return false; }
    }
    true
}

/// Unified search: tries binary index first, falls back to cached corpus scan.
/// Tag-filtered queries use index path when tag is in top-32 bitmap.
/// full_body=false uses index snippets only (no data.log I/O) for brief/medium.
pub fn search_scored(dir: &Path, terms: &[String], filter: &Filter, limit: Option<usize>,
                     index_data: Option<&[u8]>, full_body: bool)
    -> Result<(Vec<ScoredResult>, bool), String>
{
    if terms.is_empty() {
        return score_on_cache(dir, terms, filter, limit);
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
            if let Ok(result) = score_via_index(dir, data, terms, filter, limit, full_body) {
                return Ok(result);
            }
        }
    }

    // Fallback: score on borrowed cache entries (no clone storm)
    score_on_cache(dir, terms, filter, limit)
}

/// Score using binary inverted index with FilterPred for pre-scoring elimination.
fn score_via_index(dir: &Path, index_data: &[u8], terms: &[String],
                   filter: &Filter, limit: Option<usize>, full_body: bool)
    -> Result<(Vec<ScoredResult>, bool), String>
{
    let pred = build_filter_pred(index_data, filter);
    let index_limit = limit.unwrap_or(20);
    let query_str = terms.join(" ");
    let hits = crate::binquery::search_v2_filtered(index_data, &query_str, &pred, index_limit)?;

    if hits.is_empty() && filter.mode == SearchMode::And && terms.len() >= 2 {
        let or_hits = crate::binquery::search_v2_or(index_data, &query_str, &pred, index_limit)?;
        if !or_hits.is_empty() {
            return hydrate_index_hits(dir, index_data, terms, &or_hits, true, full_body);
        }
        return Ok((Vec::new(), false));
    }

    hydrate_index_hits(dir, index_data, terms, &hits, false, full_body)
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

/// Hydrate index hits into ScoredResults.
/// full_body=true: reads data.log for complete entry bodies (for full/grouped output).
/// full_body=false: uses index snippets + tag bitmap only (zero data.log I/O).
fn hydrate_index_hits(dir: &Path, index_data: &[u8], terms: &[String],
                      hits: &[crate::binquery::SearchHit], fallback: bool, full_body: bool)
    -> Result<(Vec<ScoredResult>, bool), String>
{
    if hits.is_empty() { return Ok((Vec::new(), false)); }

    let mut name_cache: FxHashMap<u16, String> = FxHashMap::default();
    // Only open data.log when full body is needed
    let mut log_file = if full_body {
        let log_path = crate::config::log_path(dir);
        Some(std::fs::File::open(&log_path).map_err(|e| format!("open data.log: {e}"))?)
    } else { None };
    let mut results = Vec::with_capacity(hits.len());

    for hit in hits {
        let topic_name = match name_cache.get(&hit.topic_id) {
            Some(n) => n.clone(),
            None => match crate::binquery::topic_name(index_data, hit.topic_id) {
                Ok(n) => { name_cache.insert(hit.topic_id, n.clone()); n }
                Err(_) => continue,
            },
        };
        let mut score = hit.score;

        // Topic-name boost — topic names are already lowercase (config::sanitize_topic)
        if terms.iter().any(|t| topic_name.contains(t.as_str())) { score *= 1.5; }

        if full_body {
            // Full hydration: read entry body from data.log
            let entry = crate::datalog::read_entry_from(log_file.as_mut().unwrap(), hit.log_offset)
                .unwrap_or(crate::datalog::LogEntry {
                    offset: hit.log_offset, topic: topic_name.clone(),
                    body: String::new(), timestamp_min: hit.date_minutes,
                });
            // Tag boost from body — tags already stored lowercase
            for line in entry.body.lines() {
                if line.starts_with("[tags: ") {
                    let tag_hits = terms.iter().filter(|t| line.contains(t.as_str())).count();
                    if tag_hits > 0 { score *= 1.0 + 0.3 * tag_hits as f64; }
                    break;
                }
            }
            let date = crate::time::minutes_to_date_str(entry.timestamp_min);
            let mut lines = vec![format!("## {date}")];
            for line in entry.body.lines() { lines.push(line.to_string()); }
            results.push(ScoredResult { name: topic_name, lines: Rc::new(lines), score });
        } else {
            // Light hydration: build lines from index data only (zero data.log I/O)
            let tag_line = crate::binquery::reconstruct_tags(index_data, hit.entry_id).ok().flatten();
            // Tag boost from reconstructed bitmap tags — already lowercase
            if let Some(ref tl) = tag_line {
                let tag_hits = terms.iter().filter(|t| tl.contains(t.as_str())).count();
                if tag_hits > 0 { score *= 1.0 + 0.3 * tag_hits as f64; }
            }
            let date = crate::time::minutes_to_date_str(hit.date_minutes);
            let mut lines = vec![format!("## {date}")];
            if let Some(tl) = tag_line { lines.push(tl); }
            // Extract content from snippet (strip "[topic] date " prefix)
            let prefix = format!("[{}] {} ", topic_name, date);
            let content = hit.snippet.strip_prefix(&prefix).unwrap_or(&hit.snippet);
            if !content.is_empty() { lines.push(content.to_string()); }
            results.push(ScoredResult { name: topic_name, lines: Rc::new(lines), score });
        }
    }
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok((results, fallback))
}

/// Collect all tags from cache for no-match suggestions.
pub fn collect_all_tags(dir: &Path) -> Vec<(String, usize)> {
    crate::cache::with_corpus(dir, |cached| {
        let mut tags: FxHashMap<String, usize> = FxHashMap::default();
        for e in cached {
            for t in e.tags() {
                *tags.entry(t.to_string()).or_insert(0) += 1;
            }
        }
        let mut sorted: Vec<(String, usize)> = tags.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted
    }).unwrap_or_default()
}
