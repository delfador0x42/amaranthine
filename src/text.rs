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

/// Lowercase ASCII bytes into a String — no UTF-8 decode needed.
#[inline]
fn ascii_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        s.push(if b.is_ascii_uppercase() { (b + 32) as char } else { b as char });
    }
    s
}

/// Emit a segment: push compound parts then the full lowercase token.
#[inline]
fn emit_segment(original: &str, lower: String, tokens: &mut Vec<String>) {
    let parts = split_compound_ascii(original);
    if parts.len() > 1 {
        for part in parts {
            if part.len() >= 2 && part != lower { tokens.push(part); }
        }
    }
    tokens.push(lower);
}

/// Extract search terms: tokenize + filter stop words + dedup.
pub fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::with_capacity(8);
    for token in tokenize(query) {
        if SEARCH_STOP_WORDS.contains(&token.as_str()) { continue; }
        if !terms.contains(&token) { terms.push(token); }
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
