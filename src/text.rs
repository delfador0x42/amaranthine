//! Shared text processing: tokenization, query terms, truncation, tag parsing.
//! tokenize() is THE unified tokenizer — used by search.rs, inverted.rs, and query_terms.

/// Conservative stop words for SEARCH only. Pure function words.
/// Does NOT include technical terms like "file", "path", "type", "name".
const SEARCH_STOP_WORDS: &[&str] = &[
    "that", "this", "with", "from", "have", "been", "were", "will", "when",
    "which", "their", "there", "about", "would", "could", "should", "into",
    "also", "each", "does", "just", "more", "than", "then", "them", "some",
    "only", "other", "very", "after", "before", "most", "same", "both",
];

/// Tokenize text: split on non-alphanumeric, expand CamelCase, lowercase.
/// Used by query_terms (+ stop words), cache.rs corpus loading, and inverted.rs.
/// Uses byte-level ASCII fast path (~30% faster) with Unicode fallback.
#[inline]
pub fn tokenize(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::with_capacity(len / 6);
    let mut pos = 0;
    while pos < len {
        // Skip non-alphanumeric bytes
        while pos < len && !bytes[pos].is_ascii_alphanumeric() && bytes[pos] < 128 {
            pos += 1;
        }
        if pos >= len { break; }
        // Non-ASCII byte: fall back to Unicode segment extraction
        if bytes[pos] >= 128 {
            let start = pos;
            while pos < len && (bytes[pos] >= 128 || bytes[pos].is_ascii_alphanumeric()) {
                pos += 1;
            }
            let segment = &text[start..pos];
            let lower = segment.to_lowercase();
            if lower.len() >= 2 { emit_segment(segment, lower, &mut tokens); }
            continue;
        }
        // ASCII fast path: scan alphanumeric bytes
        let start = pos;
        while pos < len && bytes[pos].is_ascii_alphanumeric() {
            pos += 1;
        }
        let seg = &bytes[start..pos];
        if seg.len() < 2 { continue; }
        // Lowercase via byte ops (no UTF-8 decode)
        let lower = ascii_lower(seg);
        let segment = &text[start..pos];
        emit_segment(segment, lower, &mut tokens);
    }
    tokens
}

/// Lowercase ASCII bytes into a String — memcpy + in-place lowercase.
#[inline]
fn ascii_lower(bytes: &[u8]) -> String {
    let mut v = bytes.to_vec();
    v.make_ascii_lowercase();
    // Safety: input is ASCII (caller verifies), lowercasing preserves ASCII validity.
    unsafe { String::from_utf8_unchecked(v) }
}

/// Emit a segment: push compound parts then the full lowercase token.
/// Fast path: skip split_compound_ascii for non-CamelCase words (~80% of tokens).
#[inline]
fn emit_segment(original: &str, lower: String, tokens: &mut Vec<String>) {
    let bytes = original.as_bytes();
    if bytes.len() < 2 || !bytes[1..].iter().any(|b| b.is_ascii_uppercase()) {
        tokens.push(lower);
        return;
    }
    let parts = split_compound_ascii(original);
    if parts.len() > 1 {
        for part in parts {
            if part.len() >= 2 && part != lower { tokens.push(part); }
        }
    }
    tokens.push(lower);
}

/// Build tf_map directly during tokenization — no intermediate Vec<String>.
/// Only allocates String keys for unique tokens (first occurrence).
/// Reuses a stack buffer for ASCII lowercasing (~30% of tokens are repeats → zero alloc).
pub fn tokenize_into_tfmap(text: &str, tf_map: &mut crate::fxhash::FxHashMap<String, usize>) -> usize {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut word_count = 0usize;
    let mut pos = 0;
    let mut lower_buf = Vec::with_capacity(32);
    while pos < len {
        while pos < len && !bytes[pos].is_ascii_alphanumeric() && bytes[pos] < 128 { pos += 1; }
        if pos >= len { break; }
        if bytes[pos] >= 128 {
            let start = pos;
            while pos < len && (bytes[pos] >= 128 || bytes[pos].is_ascii_alphanumeric()) { pos += 1; }
            let segment = &text[start..pos];
            let lower = segment.to_lowercase();
            if lower.len() >= 2 {
                word_count += emit_segment_tfmap(segment, &lower, tf_map);
            }
            continue;
        }
        let start = pos;
        while pos < len && bytes[pos].is_ascii_alphanumeric() { pos += 1; }
        let seg = &bytes[start..pos];
        if seg.len() < 2 { continue; }
        // Lowercase into reusable buffer (no heap alloc)
        lower_buf.clear();
        lower_buf.extend_from_slice(seg);
        lower_buf.make_ascii_lowercase();
        let lower_str = unsafe { std::str::from_utf8_unchecked(&lower_buf) };
        // CamelCase splitting
        if seg[1..].iter().any(|b| b.is_ascii_uppercase()) {
            let original = &text[start..pos];
            let parts = split_compound_ascii(original);
            if parts.len() > 1 {
                for part in &parts {
                    if part.len() >= 2 && part != lower_str {
                        word_count += 1;
                        if let Some(c) = tf_map.get_mut(part.as_str()) { *c += 1; }
                        else { tf_map.insert(part.clone(), 1); }
                    }
                }
            }
        }
        word_count += 1;
        // HashMap lookup with &str, only allocate String on first occurrence
        if let Some(c) = tf_map.get_mut(lower_str) { *c += 1; }
        else { tf_map.insert(lower_str.to_string(), 1); }
    }
    word_count
}

/// Emit a CamelCase/Unicode segment directly into tf_map. Returns token count.
#[inline]
fn emit_segment_tfmap(original: &str, lower: &str, tf_map: &mut crate::fxhash::FxHashMap<String, usize>) -> usize {
    let bytes = original.as_bytes();
    let mut count = 0;
    if bytes.len() >= 2 && bytes[1..].iter().any(|b| b.is_ascii_uppercase()) {
        let parts = split_compound_ascii(original);
        if parts.len() > 1 {
            for part in &parts {
                if part.len() >= 2 && part != lower {
                    count += 1;
                    if let Some(c) = tf_map.get_mut(part.as_str()) { *c += 1; }
                    else { tf_map.insert(part.clone(), 1); }
                }
            }
        }
    }
    count += 1;
    if let Some(c) = tf_map.get_mut(lower) { *c += 1; }
    else { tf_map.insert(lower.to_string(), 1); }
    count
}

/// Extract search terms: tokenize + filter stop words + dedup.
/// Uses FxHashSet for O(1) dedup instead of O(n) Vec::contains.
pub fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::with_capacity(8);
    let mut seen = crate::fxhash::FxHashSet::default();
    for token in tokenize(query) {
        if SEARCH_STOP_WORDS.contains(&token.as_str()) { continue; }
        if seen.insert(token.clone()) { terms.push(token); }
    }
    terms
}

/// Split CamelCase and snake_case/kebab-case into component words.
/// Uses byte-level scanning for ASCII content.
fn split_compound_ascii(s: &str) -> Vec<String> {
    let mut parts = Vec::with_capacity(4);
    for segment in s.split(|c: char| c == '_' || c == '-') {
        if segment.is_empty() { continue; }
        let bytes = segment.as_bytes();
        if bytes.iter().all(|b| b.is_ascii()) {
            // ASCII fast path: detect uppercase transitions on bytes
            let mut start = 0;
            for i in 1..bytes.len() {
                if bytes[i].is_ascii_uppercase() {
                    if i > start {
                        parts.push(ascii_lower(&bytes[start..i]));
                    }
                    start = i;
                }
            }
            if bytes.len() > start {
                parts.push(ascii_lower(&bytes[start..]));
            }
        } else {
            // Unicode fallback
            let mut current = String::new();
            let chars: Vec<char> = segment.chars().collect();
            for i in 0..chars.len() {
                if i > 0 && chars[i].is_uppercase() {
                    if !current.is_empty() { parts.push(current.to_lowercase()); current = String::new(); }
                }
                current.push(chars[i]);
            }
            if !current.is_empty() { parts.push(current.to_lowercase()); }
        }
    }
    parts
}

/// Truncate a string to max bytes at a char boundary.
#[inline]
pub fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

/// Check if a line is metadata (tags, source, type, modified, etc.).
/// Fast reject: all metadata lines start with '['.
#[inline]
pub fn is_metadata_line(line: &str) -> bool {
    if !line.starts_with('[') { return false; }
    line.starts_with("[tags:") || line.starts_with("[source:")
        || line.starts_with("[type:") || line.starts_with("[modified:")
        || line.starts_with("[tier:") || line.starts_with("[confidence:")
        || line.starts_with("[links:") || line.starts_with("[linked from:")
}

/// All metadata extracted from an entry body in a single pass.
pub struct EntryMetadata {
    pub source: Option<String>,
    pub tags: Vec<String>,
    pub confidence: f64,
    pub links: Vec<(String, usize)>,
}

/// Extract all metadata from entry body in one scan.
/// Replaces 4 separate line scans (source, tags, confidence, links).
pub fn extract_all_metadata(body: &str) -> EntryMetadata {
    let mut source = None;
    let mut tags = Vec::new();
    let mut confidence = 1.0;
    let mut links = Vec::new();

    for line in body.lines() {
        if !line.starts_with('[') { continue; }
        if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
            tags = inner.split(',').map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty()).collect();
        } else if let Some(s) = line.strip_prefix("[source: ").and_then(|s| s.strip_suffix(']')) {
            source = Some(s.trim().to_string());
        } else if let Some(c) = line.strip_prefix("[confidence: ")
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|s| s.trim().parse::<f64>().ok()) {
            confidence = c;
        } else if let Some(inner) = line.strip_prefix("[links: ").and_then(|s| s.strip_suffix(']')) {
            links = inner.split_whitespace()
                .filter_map(|pair| {
                    let (topic, idx) = pair.rsplit_once(':')?;
                    Some((topic.to_string(), idx.parse().ok()?))
                })
                .collect();
        }
    }

    EntryMetadata { source, tags, confidence, links }
}

/// Extract [source: path/to/file] from entry body text.
pub fn extract_source(body: &str) -> Option<String> {
    body.lines()
        .find_map(|l| l.strip_prefix("[source: ").and_then(|s| s.strip_suffix(']')))
        .map(|s| s.trim().to_string())
}

/// Parse raw tags line "[tags: a, b, c]" → vec!["a", "b", "c"].
/// Accepts CachedEntry.tags_raw or any "[tags: ...]" line.
pub fn parse_tags_raw(raw: Option<&str>) -> Vec<&str> {
    raw.and_then(|line| line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')))
        .map(|inner| inner.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default()
}

/// Extract [tags: ...] from entry lines, formatted as #tag1 #tag2.
pub fn extract_tags(lines: &[impl AsRef<str>]) -> Option<String> {
    for line in lines {
        if let Some(inner) = line.as_ref().strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
            let tags: Vec<&str> = inner.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
            if !tags.is_empty() { return Some(tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")); }
        }
    }
    None
}
