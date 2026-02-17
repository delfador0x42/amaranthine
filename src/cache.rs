//! In-memory corpus cache with mtime invalidation.
//! Eliminates file I/O on the hot search path.
//! Files re-read only when their mtime changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use std::{fs, fmt};

/// A single cached entry (one ## section from a topic file).
pub struct CachedEntry {
    pub lines: Vec<String>,     // original lines (for display)
    pub text_lower: String,     // pre-lowercased joined text (for search)
    pub word_count: usize,      // pre-computed for BM25 avgdl
}

/// All cached entries from one topic file.
struct CachedFile {
    mtime: SystemTime,
    entries: Vec<CachedEntry>,
}

/// Global corpus cache: path → (mtime, parsed entries).
pub struct CorpusCache {
    files: HashMap<PathBuf, CachedFile>,
}

static CACHE: Mutex<Option<CorpusCache>> = Mutex::new(None);

impl CorpusCache {
    fn new() -> Self { Self { files: HashMap::new() } }

    /// Refresh stale files, return all entries grouped by topic name.
    fn refresh<'a>(&'a mut self, paths: &'a [PathBuf]) -> Vec<(&'a str, &'a [CachedEntry])> {
        // Remove files that no longer exist in the path list
        self.files.retain(|p, _| paths.contains(p));

        for path in paths {
            let cur_mtime = fs::metadata(path)
                .and_then(|m| m.modified()).ok();
            let stale = match (self.files.get(path), cur_mtime) {
                (Some(cf), Some(m)) => cf.mtime != m,
                (None, _) => true,
                _ => true,
            };
            if stale {
                if let Ok(content) = fs::read_to_string(path) {
                    let entries = parse_entries(&content);
                    self.files.insert(path.clone(), CachedFile {
                        mtime: cur_mtime.unwrap_or(SystemTime::UNIX_EPOCH),
                        entries,
                    });
                }
            }
        }

        paths.iter().filter_map(|p| {
            let cf = self.files.get(p)?;
            let name = p.file_stem()?.to_str()?;
            Some((name, cf.entries.as_slice()))
        }).collect()
    }

    /// Invalidate a single file (after store/edit/delete).
    fn invalidate(&mut self, path: &Path) {
        self.files.remove(path);
    }
}

/// Parse a markdown file into cached entries.
fn parse_entries(content: &str) -> Vec<CachedEntry> {
    let mut entries = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in content.lines() {
        if crate::search::is_entry_header(line) && !current.is_empty() {
            entries.push(build_cached(&current));
            current = Vec::new();
        }
        current.push(line);
    }
    if current.iter().any(|l| crate::search::is_entry_header(l)) {
        entries.push(build_cached(&current));
    }
    entries
}

fn build_cached(lines: &[&str]) -> CachedEntry {
    let text_lower: String = lines.iter()
        .map(|l| l.to_lowercase()).collect::<Vec<_>>().join("\n");
    let word_count = text_lower.split_whitespace().count();
    CachedEntry {
        lines: lines.iter().map(|s| s.to_string()).collect(),
        text_lower,
        word_count,
    }
}

// --- Public API (acquires Mutex) ---

/// Get cached entries for the given paths. Refreshes stale files.
/// Returns (topic_name, entries) pairs. Borrows are short-lived.
/// Caller provides a closure to process entries while lock is held.
pub fn with_corpus<F, R>(paths: &[PathBuf], f: F) -> R
where F: FnOnce(Vec<(&str, &[CachedEntry])>) -> R {
    let mut guard = CACHE.lock().unwrap();
    let cache = guard.get_or_insert_with(CorpusCache::new);
    let groups = cache.refresh(paths);
    f(groups)
}

/// Invalidate cache for a specific topic file.
pub fn invalidate(path: &Path) {
    if let Ok(mut guard) = CACHE.lock() {
        if let Some(cache) = guard.as_mut() {
            cache.invalidate(path);
        }
    }
}

/// Invalidate all cached files.
pub fn invalidate_all() {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = None;
    }
}

/// Cache stats for diagnostics.
pub struct CacheStats {
    pub files: usize,
    pub entries: usize,
}

impl fmt::Display for CacheStats {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "cache: {} files, {} entries", self.files, self.entries)
    }
}

pub fn stats() -> CacheStats {
    let guard = CACHE.lock().unwrap();
    match guard.as_ref() {
        Some(c) => CacheStats {
            files: c.files.len(),
            entries: c.files.values().map(|f| f.entries.len()).sum(),
        },
        None => CacheStats { files: 0, entries: 0 },
    }
}
