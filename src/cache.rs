//! In-memory corpus cache with data.log mtime invalidation.
//! Eliminates file I/O + tokenization on repeated corpus-path searches.
//! Cache holds pre-tokenized entries; filters applied at query time.

use crate::fxhash::FxHashMap;
use crate::intern::InternedStr;
use std::sync::Mutex;
use std::time::SystemTime;
use std::path::Path;

pub struct CachedEntry {
    pub topic: InternedStr,
    pub body: String,
    pub timestamp_min: i32,
    pub offset: u32,
    pub tf_map: FxHashMap<String, usize>,
    pub word_count: usize,
    pub tags: Vec<String>, // pre-parsed from [tags: ...] metadata — no re-parsing on access
    pub source: Option<String>,
    pub confidence: f64, // 0.0-1.0, default 1.0; explicit from [confidence:] metadata
    pub links: Vec<(String, usize)>, // (topic, entry_index) from [links:] metadata
    pub snippet: String, // precomputed "[topic] date content" for index builder
}

impl CachedEntry {
    /// Check if entry has a specific tag. O(tags) slice scan, no re-parsing.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
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
    for e in raw_entries {
        let topic = match interned.iter().find(|t| t.as_str() == e.topic.as_str()) {
            Some(t) => t.clone(),
            None => { let t = InternedStr::new(&e.topic); interned.push(t.clone()); t }
        };
        // Single-pass metadata extraction: replaces 4 separate line scans
        let meta = crate::text::extract_all_metadata(&e.body);
        let mut tf_map: FxHashMap<String, usize> = crate::fxhash::map_with_capacity(32);
        let word_count = crate::text::tokenize_into_tfmap(&e.body, &mut tf_map);
        let snippet = build_snippet(topic.as_str(), e.timestamp_min, &e.body);
        entries.push(CachedEntry {
            topic, body: e.body, timestamp_min: e.timestamp_min, offset: e.offset,
            tf_map, word_count, tags: meta.tags, source: meta.source,
            confidence: meta.confidence, links: meta.links, snippet,
        });
    }

    let result = f(&entries);
    *guard = Some(CachedCorpus { mtime: cur_mtime, entries });
    Ok(result)
}

/// Append a new entry to the in-memory cache and update mtime.
/// Avoids cache invalidation after store (eliminates double corpus load).
/// No-op if cache is empty (cold start — next read will do full load).
pub fn append_to_cache(dir: &Path, topic: &str, body: &str, ts_min: i32, offset: u32) {
    let log_path = crate::config::log_path(dir);
    let cur_mtime = std::fs::metadata(&log_path)
        .and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut guard = match CACHE.lock() { Ok(g) => g, Err(_) => return };
    let cache = match guard.as_mut() { Some(c) => c, None => return };
    let topic_interned = cache.entries.iter()
        .find(|e| e.topic.as_str() == topic)
        .map(|e| e.topic.clone())
        .unwrap_or_else(|| InternedStr::new(topic));
    let meta = crate::text::extract_all_metadata(body);
    let mut tf_map = crate::fxhash::map_with_capacity(32);
    let word_count = crate::text::tokenize_into_tfmap(body, &mut tf_map);
    let snippet = build_snippet(topic, ts_min, body);
    cache.entries.push(CachedEntry {
        topic: topic_interned, body: body.to_string(), timestamp_min: ts_min,
        offset, tf_map, word_count, tags: meta.tags, source: meta.source,
        confidence: meta.confidence, links: meta.links, snippet,
    });
    cache.mtime = cur_mtime;
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

/// Build index snippet: "[topic] date content_preview". Computed once, reused by rebuild.
fn build_snippet(topic: &str, ts_min: i32, body: &str) -> String {
    let date = crate::time::minutes_to_date_str(ts_min);
    let lines: Vec<&str> = body.lines()
        .filter(|l| !crate::text::is_metadata_line(l) && !l.trim().is_empty())
        .take(2).collect();
    format!("[{}] {} {}", topic, date, crate::text::truncate(&lines.join(" "), 120))
}
