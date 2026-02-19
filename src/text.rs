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
/// Used by query_terms (+ stop words), search.rs load_corpus, and inverted.rs.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for segment in text.split(|c: char| !c.is_alphanumeric()) {
        if segment.is_empty() { continue; }
        let lower = segment.to_lowercase();
        if lower.len() < 2 { continue; }
        tokens.push(lower.clone());
        let parts = split_compound(segment);
        if parts.len() > 1 {
            for part in parts {
                if part.len() >= 2 && part != lower { tokens.push(part); }
            }
        }
    }
    tokens
}

/// Extract search terms: tokenize + filter stop words + dedup.
pub fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for token in tokenize(query) {
        if SEARCH_STOP_WORDS.contains(&token.as_str()) { continue; }
        if !terms.contains(&token) { terms.push(token); }
    }
    terms
}

/// Split CamelCase and snake_case/kebab-case into component words.
fn split_compound(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for segment in s.split(|c: char| c == '_' || c == '-') {
        if segment.is_empty() { continue; }
        let mut current = String::new();
        let chars: Vec<char> = segment.chars().collect();
        for i in 0..chars.len() {
            if i > 0 && chars[i].is_uppercase() { if !current.is_empty() { parts.push(current.to_lowercase()); current = String::new(); } }
            current.push(chars[i]);
        }
        if !current.is_empty() { parts.push(current.to_lowercase()); }
    }
    parts
}

/// Truncate a string to max bytes at a char boundary.
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
