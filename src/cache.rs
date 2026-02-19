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
        entries.push(CachedEntry {
            topic,
            body: e.body.clone(),
            timestamp_min: e.timestamp_min,
            offset: e.offset,
            tokens, token_set, tf_map, word_count, tags_raw,
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
