//! In-memory corpus cache with data.log mtime invalidation.
//! Eliminates file I/O + tokenization on repeated corpus-path searches.
//! Cache holds pre-tokenized entries; filters applied at query time.

use crate::fxhash::{FxHashSet, FxHashMap};
use crate::intern::InternedStr;
use std::sync::Mutex;
use std::time::SystemTime;
use std::path::Path;

pub struct CachedEntry {
    pub topic: InternedStr,
    pub body: String,
    pub timestamp_min: i32,
    pub offset: u32,
    pub tokens: Vec<String>,
    pub token_set: FxHashSet<String>,
    pub tf_map: FxHashMap<String, usize>,
    pub word_count: usize,
    pub tags_raw: Option<String>,
    pub confidence: f64, // 0.0-1.0, default 1.0; explicit from [confidence:] metadata
    pub links: Vec<(String, usize)>, // (topic, entry_index) from [links:] metadata
}

impl CachedEntry {
    /// Parse tags from cached tags_raw field.
    pub fn tags(&self) -> Vec<&str> {
        crate::text::parse_tags_raw(self.tags_raw.as_deref())
    }
    /// Check if entry has a specific tag (tags are already lowercase).
    pub fn has_tag(&self, tag: &str) -> bool {
        crate::text::parse_tags_raw(self.tags_raw.as_deref())
            .iter().any(|t| *t == tag)
    }
    /// Format timestamp as "YYYY-MM-DD HH:MM".
    pub fn date_str(&self) -> String {
        crate::time::minutes_to_date_str(self.timestamp_min)
    }
    /// Day number since epoch (minutes / 1440).
    pub fn day(&self) -> i64 {
        self.timestamp_min as i64 / 1440
    }
    /// Days since entry was created.
    pub fn days_old(&self, now_days: i64) -> i64 {
        now_days - self.day()
    }
    /// First non-metadata content line of entry body.
    pub fn preview(&self) -> &str {
        crate::compress::first_content(&self.body)
    }
    /// Confidence as u8 (0-255) for binary index.
    pub fn confidence_u8(&self) -> u8 {
        (self.confidence.clamp(0.0, 1.0) * 255.0) as u8
    }
    /// Whether this entry has narrative links to other entries.
    pub fn has_links(&self) -> bool {
        !self.links.is_empty()
    }
}

struct CachedCorpus {
    mtime: SystemTime,
    entries: Vec<CachedEntry>,
}

static CACHE: Mutex<Option<CachedCorpus>> = Mutex::new(None);

/// Invalidate cache (call after any write to data.log).
pub fn invalidate() {
    if let Ok(mut g) = CACHE.lock() { *g = None; }
}

/// Access cached corpus via closure. Reloads from data.log only if mtime changed.
/// The closure receives all entries (unfiltered). Filter in the closure.
pub fn with_corpus<F, R>(dir: &Path, f: F) -> Result<R, String>
where F: FnOnce(&[CachedEntry]) -> R {
    let log_path = crate::config::log_path(dir);
    let cur_mtime = std::fs::metadata(&log_path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut guard = CACHE.lock().map_err(|e| e.to_string())?;

    // Check if cache is fresh
    if let Some(ref cache) = *guard {
        if cache.mtime == cur_mtime {
            return Ok(f(&cache.entries));
        }
    }

    // Cache miss: reload from data.log
    let raw_entries = crate::datalog::iter_live(&log_path)?;
    let mut entries = Vec::with_capacity(raw_entries.len());
    // Intern topic names: ~45 unique across ~1000 entries → 955 fewer heap allocs
    let mut interned: Vec<InternedStr> = Vec::with_capacity(64);
    for e in &raw_entries {
        let topic = match interned.iter().find(|t| t.as_str() == e.topic.as_str()) {
            Some(t) => t.clone(),
            None => { let t = InternedStr::new(&e.topic); interned.push(t.clone()); t }
        };
        let tokens = crate::text::tokenize(&e.body);
        let word_count = tokens.len();
        // Single-pass: build tf_map from tokens (one clone per unique token)
        let mut tf_map: FxHashMap<String, usize> = crate::fxhash::map_with_capacity(word_count / 2);
        for t in &tokens { *tf_map.entry(t.clone()).or_insert(0) += 1; }
        // Derive token_set from tf_map keys (zero extra String allocs)
        let token_set: FxHashSet<String> = tf_map.keys().cloned().collect();
        let tags_raw = e.body.lines()
            .find(|l| l.starts_with("[tags: "))
            .map(|l| l.to_string());
        let confidence = e.body.lines()
            .find_map(|l| l.strip_prefix("[confidence: ")
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.trim().parse::<f64>().ok()))
            .unwrap_or(1.0);
        let links = parse_links(&e.body);
        entries.push(CachedEntry {
            topic,
            body: e.body.clone(),
            timestamp_min: e.timestamp_min,
            offset: e.offset,
            tokens, token_set, tf_map, word_count, tags_raw, confidence, links,
        });
    }

    let result = f(&entries);
    *guard = Some(CachedCorpus { mtime: cur_mtime, entries });
    Ok(result)
}

pub struct CacheStats {
    pub entries: usize,
    pub cached: bool,
}

pub fn stats() -> CacheStats {
    let guard = CACHE.lock().unwrap();
    match guard.as_ref() {
        Some(c) => CacheStats { entries: c.entries.len(), cached: true },
        None => CacheStats { entries: 0, cached: false },
    }
}

/// Parse `[links: topic:idx topic:idx]` metadata from entry body.
fn parse_links(body: &str) -> Vec<(String, usize)> {
    body.lines()
        .find_map(|l| l.strip_prefix("[links: ").and_then(|s| s.strip_suffix(']')))
        .map(|inner| {
            inner.split_whitespace()
                .filter_map(|pair| {
                    let (topic, idx) = pair.rsplit_once(':')?;
                    Some((topic.to_string(), idx.parse().ok()?))
                })
                .collect()
        })
        .unwrap_or_default()
}
