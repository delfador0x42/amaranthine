//! In-memory corpus cache with data.log mtime invalidation.
//! Eliminates file I/O + tokenization on repeated corpus-path searches.
//! Cache holds pre-tokenized entries; metadata parsed lazily on first access.

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
    pub snippet: String,
    meta: std::cell::OnceCell<crate::text::EntryMetadata>,
}

impl CachedEntry {
    /// Lazily parse metadata from body on first access.
    fn meta(&self) -> &crate::text::EntryMetadata {
        self.meta.get_or_init(|| crate::text::extract_all_metadata(&self.body))
    }
    /// Tags from [tags: ...] metadata. Lazy: parsed on first access.
    pub fn tags(&self) -> &[String] { &self.meta().tags }
    /// Source path from [source: ...] metadata. Lazy.
    pub fn source(&self) -> Option<&str> { self.meta().source.as_deref() }
    /// Confidence value (0.0-1.0, default 1.0). Lazy.
    pub fn confidence(&self) -> f64 { self.meta().confidence }
    /// Narrative links from [links: ...] metadata. Lazy.
    pub fn links(&self) -> &[(String, usize)] { &self.meta().links }
    /// Check if entry has a specific tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags().iter().any(|t| t == tag)
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
        (self.confidence().clamp(0.0, 1.0) * 255.0) as u8
    }
    /// Whether this entry has narrative links to other entries.
    pub fn has_links(&self) -> bool {
        !self.links().is_empty()
    }
}

struct CachedCorpus {
    mtime: SystemTime,
    entries: Vec<CachedEntry>,
    intern_pool: FxHashMap<String, InternedStr>,
}

/// Invalidate cache (call after any write to data.log).
pub fn invalidate() {
    if let Ok(mut g) = CACHE.lock() { *g = None; }
}

static CACHE: Mutex<Option<CachedCorpus>> = Mutex::new(None);

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

    // Cache miss: reload from data.log (metadata parsed lazily on first access)
    let raw_entries = crate::datalog::iter_live(&log_path)?;
    let mut entries = Vec::with_capacity(raw_entries.len());
    let mut intern_pool: FxHashMap<String, InternedStr> = FxHashMap::default();
    for e in raw_entries {
        let topic = match intern_pool.get(e.topic.as_str()) {
            Some(t) => t.clone(),
            None => { let t = InternedStr::new(&e.topic); intern_pool.insert(e.topic.clone(), t.clone()); t }
        };
        let mut tf_map: FxHashMap<String, usize> = crate::fxhash::map_with_capacity(32);
        let word_count = crate::text::tokenize_into_tfmap(&e.body, &mut tf_map);
        let snippet = build_snippet(topic.as_str(), e.timestamp_min, &e.body);
        entries.push(CachedEntry {
            topic, body: e.body, timestamp_min: e.timestamp_min, offset: e.offset,
            tf_map, word_count, snippet, meta: std::cell::OnceCell::new(),
        });
    }

    let result = f(&entries);
    *guard = Some(CachedCorpus { mtime: cur_mtime, entries, intern_pool });
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
    let topic_interned = match cache.intern_pool.get(topic) {
        Some(t) => t.clone(),
        None => { let t = InternedStr::new(topic); cache.intern_pool.insert(topic.to_string(), t.clone()); t }
    };
    let mut tf_map = crate::fxhash::map_with_capacity(32);
    let word_count = crate::text::tokenize_into_tfmap(body, &mut tf_map);
    let snippet = build_snippet(topic, ts_min, body);
    cache.entries.push(CachedEntry {
        topic: topic_interned, body: body.to_string(), timestamp_min: ts_min,
        offset, tf_map, word_count, snippet, meta: std::cell::OnceCell::new(),
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
/// v7.4: single allocation — direct push_str replaces format! + Vec + join (was 4 allocs).
fn build_snippet(topic: &str, ts_min: i32, body: &str) -> String {
    // Pre-size: "[topic] YYYY-MM-DD HH:MM " + ~120 chars content
    let mut buf = String::with_capacity(topic.len() + 20 + 120);
    buf.push('[');
    buf.push_str(topic);
    buf.push_str("] ");
    crate::time::minutes_to_date_str_into(ts_min, &mut buf);
    buf.push(' ');
    // Inline content preview: take first 2 non-metadata, non-empty lines joined by space
    let content_start = buf.len();
    let mut line_count = 0u8;
    for line in body.lines() {
        if crate::text::is_metadata_line(line) || line.trim().is_empty() { continue; }
        if line_count > 0 { buf.push(' '); }
        buf.push_str(line.trim());
        line_count += 1;
        if line_count >= 2 { break; }
        // Cap content at ~120 chars
        if buf.len() - content_start >= 120 { break; }
    }
    // Truncate content portion to ~120 bytes at a word boundary (char-safe).
    let content_len = buf.len() - content_start;
    if content_len > 120 {
        // Find the largest char boundary <= 120
        let mut boundary = 120;
        while boundary > 0 && !buf.is_char_boundary(content_start + boundary) {
            boundary -= 1;
        }
        let content = &buf[content_start..content_start + boundary];
        let trunc_at = content.rfind(' ').unwrap_or(boundary);
        buf.truncate(content_start + trunc_at);
    }
    buf
}
