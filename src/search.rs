use std::fmt::Write;
use std::fs;
use std::path::Path;

/// Filter options for search (date range + tag + mode)
pub struct Filter {
    pub after: Option<i64>,  // days since epoch
    pub before: Option<i64>,
    pub tag: Option<String>,
    pub mode: SearchMode,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SearchMode { And, Or }

impl Filter {
    pub fn none() -> Self { Self { after: None, before: None, tag: None, mode: SearchMode::And } }
    pub fn is_active(&self) -> bool { self.after.is_some() || self.before.is_some() || self.tag.is_some() }
}

pub fn run(dir: &Path, query: &str, plain: bool, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    search(dir, query, plain, false, limit, filter)
}

pub fn run_brief(dir: &Path, query: &str, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    search(dir, query, true, true, limit, filter)
}

pub fn run_medium(dir: &Path, query: &str, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let terms = query_terms(query);
    let files = crate::config::list_search_files(dir)?;

    // Phase 1: read + pre-filter + lowercase
    let mut corpus: Vec<PrepSection> = Vec::new();
    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        for section in parse_sections(&content) {
            if !passes_filter(&section, filter) { continue; }
            let text_lower = section.iter()
                .map(|l| l.to_lowercase()).collect::<Vec<_>>().join("\n");
            corpus.push(PrepSection {
                name: name.clone(),
                lines: section.iter().map(|s| s.to_string()).collect(),
                text_lower,
            });
        }
    }

    // Phase 2: BM25 corpus stats
    let n = corpus.len() as f64;
    let total_words: usize = corpus.iter()
        .map(|s| s.text_lower.split_whitespace().count()).sum();
    let avgdl = if corpus.is_empty() { 1.0 } else { total_words as f64 / n };
    let dfs: Vec<usize> = terms.iter()
        .map(|t| corpus.iter().filter(|s| s.text_lower.contains(t.as_str())).count())
        .collect();

    // Phase 3: match + score
    let mut mode = filter.mode;
    let mut results: Vec<ScoredResult> = Vec::new();
    for ps in &corpus {
        if matches_text(&ps.text_lower, &terms, mode) {
            let score = bm25_score(&ps.text_lower, &terms, n, avgdl, &dfs);
            results.push(ScoredResult {
                name: ps.name.clone(), section: ps.lines.clone(), score,
            });
        }
    }

    // AND→OR fallback
    let mut fallback = false;
    if results.is_empty() && mode == SearchMode::And && terms.len() >= 2 {
        mode = SearchMode::Or;
        for ps in &corpus {
            if matches_text(&ps.text_lower, &terms, mode) {
                let score = bm25_score(&ps.text_lower, &terms, n, avgdl, &dfs);
                results.push(ScoredResult {
                    name: ps.name.clone(), section: ps.lines.clone(), score,
                });
            }
        }
        fallback = !results.is_empty();
    }

    if !terms.is_empty() {
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    }

    let total = results.len();
    let show = limit.map(|l| total.min(l)).unwrap_or(total);
    let mut out = String::new();
    if fallback {
        let _ = writeln!(out, "(no exact match — showing OR results)");
    }

    // Medium format: [topic] timestamp header + first 2 content lines
    for r in results.iter().take(show) {
        let header = r.section.first().map(|s| s.as_str()).unwrap_or("??");
        let _ = writeln!(out, "  [{}] {}", r.name, header.trim_start_matches("## "));
        let mut content_lines = 0;
        for line in r.section.iter().skip(1) {
            if line.starts_with("[tags:") || line.trim().is_empty() { continue; }
            let short = truncate(line.trim(), 100);
            let _ = writeln!(out, "    {short}");
            content_lines += 1;
            if content_lines >= 2 { break; }
        }
    }

    if total == 0 {
        out.push_str(&no_match_message(query, filter, dir));
    } else if show < total {
        let _ = writeln!(out, "{total} match(es), showing {show}");
    } else {
        let _ = writeln!(out, "{total} match(es)");
    }
    Ok(out)
}

pub fn run_topics(dir: &Path, query: &str, filter: &Filter) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }
    let terms = query_terms(query);
    let files = crate::config::list_search_files(dir)?;

    let count_hits = |mode: SearchMode| -> Vec<(String, usize)> {
        let mut hits = Vec::new();
        for path in &files {
            let content = match fs::read_to_string(path) { Ok(c) => c, Err(_) => continue };
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            let sections = parse_sections(&content);
            let n = sections.iter()
                .filter(|s| passes_filter(s, filter) && matches_terms(s, &terms, mode))
                .count();
            if n > 0 { hits.push((name, n)); }
        }
        hits
    };

    let mut hits = count_hits(filter.mode);
    let mut fallback = false;
    if hits.is_empty() && filter.mode == SearchMode::And && terms.len() >= 2 {
        hits = count_hits(SearchMode::Or);
        fallback = !hits.is_empty();
    }

    let total: usize = hits.iter().map(|(_, n)| n).sum();
    let mut out = String::new();
    if hits.is_empty() {
        out.push_str(&no_match_message(query, filter, dir));
    } else {
        if fallback {
            let _ = writeln!(out, "(no exact match — showing OR results)");
        }
        for (topic, n) in &hits {
            let _ = writeln!(out, "  {topic}: {n} hit{}", if *n == 1 { "" } else { "s" });
        }
        let _ = writeln!(out, "{total} match(es) across {} topic(s)", hits.len());
    }
    Ok(out)
}

pub fn count(dir: &Path, query: &str, filter: &Filter) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }
    let terms = query_terms(query);
    let files = crate::config::list_search_files(dir)?;

    let do_count = |mode: SearchMode| -> (usize, usize) {
        let mut total = 0;
        let mut topics = 0;
        for path in &files {
            let content = match fs::read_to_string(path) { Ok(c) => c, Err(_) => continue };
            let sections = parse_sections(&content);
            let file_hits = sections.iter()
                .filter(|s| passes_filter(s, filter) && matches_terms(s, &terms, mode))
                .count();
            total += file_hits;
            if file_hits > 0 { topics += 1; }
        }
        (total, topics)
    };

    let (total, topics) = do_count(filter.mode);
    if total > 0 {
        return Ok(format!("{total} matches across {topics} topics for '{query}'"));
    }
    // AND→OR fallback
    if filter.mode == SearchMode::And && terms.len() >= 2 {
        let (total, topics) = do_count(SearchMode::Or);
        if total > 0 {
            return Ok(format!("(no exact match — OR fallback) {total} matches across {topics} topics for '{query}'"));
        }
    }
    Ok(format!("0 matches for '{query}'"))
}

/// BM25 parameters (Okapi BM25 standard values)
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const HEADER_BOOST: f64 = 2.0;

/// Scored search result for ranking.
struct ScoredResult {
    name: String,
    section: Vec<String>,
    score: f64,
}

/// Pre-processed section for BM25 corpus stats.
struct PrepSection {
    name: String,
    lines: Vec<String>,
    text_lower: String,
}

/// BM25 score: IDF × saturated TF × header boost.
fn bm25_score(text: &str, terms: &[String], n: f64, avgdl: f64, dfs: &[usize]) -> f64 {
    if terms.is_empty() { return 1.0; }
    let doc_len = text.split_whitespace().count() as f64;
    let len_norm = 1.0 - BM25_B + BM25_B * doc_len / avgdl.max(1.0);
    let header_end = text.find('\n').unwrap_or(text.len());
    let header = &text[..header_end];
    let mut score = 0.0;
    for (i, term) in terms.iter().enumerate() {
        let tf = text.split_whitespace()
            .filter(|w| w.contains(term.as_str()))
            .count() as f64;
        if tf == 0.0 { continue; }
        let df = dfs[i] as f64;
        let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
        let tf_sat = (tf * (BM25_K1 + 1.0)) / (tf + BM25_K1 * len_norm);
        let mut ts = idf * tf_sat;
        if header.contains(term.as_str()) { ts *= HEADER_BOOST; }
        score += ts;
    }
    score
}

/// Match against pre-lowercased text.
fn matches_text(text: &str, terms: &[String], mode: SearchMode) -> bool {
    if terms.is_empty() { return true; }
    match mode {
        SearchMode::And => terms.iter().all(|t| text.contains(t.as_str())),
        SearchMode::Or => terms.iter().any(|t| text.contains(t.as_str())),
    }
}

fn search(dir: &Path, query: &str, plain: bool, brief: bool, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let terms = query_terms(query);
    let files = crate::config::list_search_files(dir)?;

    // Phase 1: read all files once, pre-filter, pre-compute lowercase
    let mut corpus: Vec<PrepSection> = Vec::new();
    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        for section in parse_sections(&content) {
            if !passes_filter(&section, filter) { continue; }
            let text_lower = section.iter()
                .map(|l| l.to_lowercase()).collect::<Vec<_>>().join("\n");
            corpus.push(PrepSection {
                name: name.clone(),
                lines: section.iter().map(|s| s.to_string()).collect(),
                text_lower,
            });
        }
    }

    // Phase 2: BM25 corpus statistics
    let n = corpus.len() as f64;
    let total_words: usize = corpus.iter()
        .map(|s| s.text_lower.split_whitespace().count()).sum();
    let avgdl = if corpus.is_empty() { 1.0 } else { total_words as f64 / n };
    let dfs: Vec<usize> = terms.iter()
        .map(|t| corpus.iter().filter(|s| s.text_lower.contains(t.as_str())).count())
        .collect();

    // Phase 3: match + BM25 score
    let mut mode = filter.mode;
    let mut results: Vec<ScoredResult> = Vec::new();

    for ps in &corpus {
        if matches_text(&ps.text_lower, &terms, mode) {
            let score = bm25_score(&ps.text_lower, &terms, n, avgdl, &dfs);
            results.push(ScoredResult {
                name: ps.name.clone(),
                section: ps.lines.clone(),
                score,
            });
        }
    }

    // Progressive fallback: AND → OR if no results
    let mut fallback_note = String::new();
    if results.is_empty() && mode == SearchMode::And && terms.len() >= 2 {
        mode = SearchMode::Or;
        for ps in &corpus {
            if matches_text(&ps.text_lower, &terms, mode) {
                let score = bm25_score(&ps.text_lower, &terms, n, avgdl, &dfs);
                results.push(ScoredResult {
                    name: ps.name.clone(),
                    section: ps.lines.clone(),
                    score,
                });
            }
        }
        if !results.is_empty() {
            fallback_note = format!("(no exact match — showing {} OR results)\n", results.len());
        }
    }

    // Sort by BM25 score descending
    if !terms.is_empty() {
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    }

    let total = results.len();
    let show = match limit {
        Some(lim) => results.len().min(lim),
        None => results.len(),
    };

    let mut out = String::new();
    if !fallback_note.is_empty() {
        out.push_str(&fallback_note);
    }

    let mut last_file = String::new();

    for result in results.iter().take(show) {
        if brief {
            let section_refs: Vec<&str> = result.section.iter().map(|s| s.as_str()).collect();
            if terms.is_empty() {
                if let Some(hit) = section_refs.iter().find(|l| !l.starts_with("## ") && !l.starts_with("[tags:") && !l.trim().is_empty()) {
                    let short = truncate(hit.trim(), 80);
                    let _ = writeln!(out, "  [{}] {short}", result.name);
                }
            } else if let Some(hit) = section_refs.iter().find(|l| terms.iter().any(|t| l.to_lowercase().contains(t.as_str()))) {
                let trimmed = hit.trim_start_matches("- ").trim();
                let short = truncate(trimmed, 80);
                let _ = writeln!(out, "  [{}] {short}", result.name);
            }
        } else {
            if result.name != last_file {
                if plain {
                    let _ = writeln!(out, "\n--- {}.md ---", result.name);
                } else {
                    let _ = writeln!(out, "\n\x1b[1;36m--- {}.md ---\x1b[0m", result.name);
                }
                last_file = result.name.clone();
            }
            for line in &result.section {
                if !terms.is_empty() && terms.iter().any(|t| line.to_lowercase().contains(t.as_str())) {
                    if plain {
                        let _ = writeln!(out, "> {line}");
                    } else {
                        let _ = writeln!(out, "\x1b[1;33m{line}\x1b[0m");
                    }
                } else {
                    let _ = writeln!(out, "{line}");
                }
            }
            let _ = writeln!(out);
        }
    }

    if total == 0 {
        out.push_str(&no_match_message(query, filter, dir));
    } else if show < total {
        let _ = writeln!(out, "(showing {show} of {total} matches, limit applied)");
    } else if brief {
        let _ = writeln!(out, "{total} match(es)");
    } else {
        let _ = writeln!(out, "{total} matching section(s)");
    }
    Ok(out)
}

/// Build a helpful "no matches" message. If tag filter is active and tag
/// doesn't exist, tell the user. Suggest existing tags if possible.
fn no_match_message(query: &str, filter: &Filter, dir: &Path) -> String {
    let mut msg = String::new();
    if let Some(ref tag) = filter.tag {
        let existing = collect_all_tags(dir);
        if !existing.iter().any(|(t, _)| t == &tag.to_lowercase()) {
            let _ = writeln!(msg, "tag '{}' not found", tag);
            if !existing.is_empty() {
                // Suggest similar tags
                let similar: Vec<&str> = existing.iter()
                    .filter(|(t, _)| t.contains(&tag.to_lowercase()) || tag.to_lowercase().contains(t.as_str()))
                    .map(|(t, _)| t.as_str())
                    .take(5)
                    .collect();
                if !similar.is_empty() {
                    let _ = writeln!(msg, "  similar: {}", similar.join(", "));
                } else {
                    let top: Vec<&str> = existing.iter().take(8).map(|(t, _)| t.as_str()).collect();
                    let _ = writeln!(msg, "  existing tags: {}", top.join(", "));
                }
            }
            return msg;
        }
        let _ = writeln!(msg, "no entries with tag '{}' match '{}'", tag, query);
    } else {
        let _ = writeln!(msg, "no matches for '{query}'");
    }
    msg
}

/// Collect all tags across all topic files, sorted by frequency.
fn collect_all_tags(dir: &Path) -> Vec<(String, usize)> {
    let files = match crate::config::list_topic_files(dir) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut tags: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for path in &files {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
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
}

/// Check if a section passes date and tag filters.
fn passes_filter(section: &[&str], filter: &Filter) -> bool {
    // Date filter: extract from "## YYYY-MM-DD HH:MM" header
    if filter.after.is_some() || filter.before.is_some() {
        let date = section.first()
            .and_then(|h| h.strip_prefix("## "))
            .and_then(crate::time::parse_date_days);
        match date {
            Some(d) => {
                if let Some(after) = filter.after {
                    if d < after { return false; }
                }
                if let Some(before) = filter.before {
                    if d > before { return false; }
                }
            }
            None => return false, // no parseable date → skip when date filter active
        }
    }
    // Tag filter: look for "[tags: ...]" line in section
    if let Some(ref tag) = filter.tag {
        let tag_lower = tag.to_lowercase();
        let has_tag = section.iter().any(|line| {
            if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                inner.split(',').any(|t| t.trim().to_lowercase() == tag_lower)
            } else {
                false
            }
        });
        if !has_tag { return false; }
    }
    true
}

/// Split query into lowercase terms. Splits CamelCase and snake_case into components.
fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for word in query.split_whitespace() {
        let lower = word.to_lowercase();
        terms.push(lower.clone());
        // Split CamelCase BEFORE lowercasing: "SysctlHelper" → ["sysctl", "helper"]
        let parts = split_compound(word);
        if parts.len() > 1 {
            for part in parts {
                if part.len() >= 3 && !terms.contains(&part) {
                    terms.push(part);
                }
            }
        }
    }
    terms
}

/// Split CamelCase, snake_case, and kebab-case into component words.
fn split_compound(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    // First split on _ and -
    for segment in s.split(|c: char| c == '_' || c == '-') {
        if segment.is_empty() { continue; }
        // Then split CamelCase within each segment
        let mut current = String::new();
        let chars: Vec<char> = segment.chars().collect();
        for i in 0..chars.len() {
            if i > 0 && chars[i].is_uppercase() {
                if !current.is_empty() {
                    parts.push(current.to_lowercase());
                    current = String::new();
                }
            }
            current.push(chars[i]);
        }
        if !current.is_empty() {
            parts.push(current.to_lowercase());
        }
    }
    parts
}

/// Match terms against section content. AND requires all terms, OR requires any.
fn matches_terms(section: &[&str], terms: &[String], mode: SearchMode) -> bool {
    if terms.is_empty() { return true; }
    let combined: String = section.iter().map(|l| l.to_lowercase()).collect::<Vec<_>>().join("\n");
    match mode {
        SearchMode::And => terms.iter().all(|term| combined.contains(term.as_str())),
        SearchMode::Or => terms.iter().any(|term| combined.contains(term.as_str())),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

/// Check if a line is an entry header: "## YYYY-MM-DD" pattern.
/// Prevents "## " in entry body text from breaking section boundaries.
pub fn is_entry_header(line: &str) -> bool {
    let b = line.as_bytes();
    // "## YYYY-" = 8 chars minimum
    b.len() >= 8 && b[0] == b'#' && b[1] == b'#' && b[2] == b' '
        && b[3].is_ascii_digit() && b[4].is_ascii_digit()
        && b[5].is_ascii_digit() && b[6].is_ascii_digit() && b[7] == b'-'
}

pub fn parse_sections(content: &str) -> Vec<Vec<&str>> {
    let mut sections: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in content.lines() {
        if is_entry_header(line) && !current.is_empty() {
            sections.push(current);
            current = Vec::new();
        }
        current.push(line);
    }
    if !current.is_empty() {
        sections.push(current);
    }
    sections
}
