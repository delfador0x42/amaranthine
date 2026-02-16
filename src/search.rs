use std::fmt::Write;
use std::fs;
use std::path::Path;

/// Filter options for search (date range + tag)
pub struct Filter {
    pub after: Option<i64>,  // days since epoch
    pub before: Option<i64>,
    pub tag: Option<String>,
}

impl Filter {
    pub fn none() -> Self { Self { after: None, before: None, tag: None } }
    pub fn is_active(&self) -> bool { self.after.is_some() || self.before.is_some() || self.tag.is_some() }
}

pub fn run(dir: &Path, query: &str, plain: bool, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    search(dir, query, plain, false, limit, filter)
}

pub fn run_brief(dir: &Path, query: &str, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    search(dir, query, true, true, limit, filter)
}

pub fn run_topics(dir: &Path, query: &str, filter: &Filter) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }
    let terms = query_terms(query);
    let files = crate::config::list_search_files(dir)?;
    let mut hits: Vec<(String, usize)> = Vec::new();
    let mut total = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let sections = parse_sections(&content);
        let mut n = 0;
        for section in &sections {
            if !passes_filter(section, filter) { continue; }
            if matches_terms(section, &terms) {
                n += 1;
            }
        }
        if n > 0 {
            total += n;
            hits.push((name, n));
        }
    }

    let mut out = String::new();
    if hits.is_empty() {
        let _ = writeln!(out, "no matches for '{query}'");
    } else {
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
    let mut total = 0;
    let mut topics = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let sections = parse_sections(&content);
        let mut file_hits = 0;
        for section in &sections {
            if !passes_filter(section, filter) { continue; }
            if matches_terms(section, &terms) {
                file_hits += 1;
                total += 1;
            }
        }
        if file_hits > 0 { topics += 1; }
    }
    Ok(format!("{total} matches across {topics} topics for '{query}'"))
}

/// Scored search result for ranking.
struct ScoredResult {
    name: String,
    section: Vec<String>,  // owned lines
    score: u32,
}

/// Score a section by relevance: header matches worth more, count occurrences.
fn score_section(section: &[&str], terms: &[String]) -> u32 {
    if terms.is_empty() { return 1; }
    let mut score: u32 = 0;
    for (i, line) in section.iter().enumerate() {
        let lower = line.to_lowercase();
        for term in terms {
            if lower.contains(term.as_str()) {
                // Header match (## line) = 3 points, body = 1 point
                score += if i == 0 { 3 } else { 1 };
            }
        }
    }
    score
}

fn search(dir: &Path, query: &str, plain: bool, brief: bool, limit: Option<usize>, filter: &Filter) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let terms = query_terms(query);
    let files = crate::config::list_search_files(dir)?;
    let mut results: Vec<ScoredResult> = Vec::new();

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let sections = parse_sections(&content);

        for section in &sections {
            if !passes_filter(section, filter) { continue; }
            if matches_terms(section, &terms) {
                let score = score_section(section, &terms);
                results.push(ScoredResult {
                    name: name.clone(),
                    section: section.iter().map(|s| s.to_string()).collect(),
                    score,
                });
            }
        }
    }

    // Sort by score descending (relevance ranking)
    if !terms.is_empty() {
        results.sort_by(|a, b| b.score.cmp(&a.score));
    }

    let total = results.len();
    let show = match limit {
        Some(lim) => results.len().min(lim),
        None => results.len(),
    };

    let mut out = String::new();
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
        if filter.is_active() {
            let _ = writeln!(out, "no matches (filters active)");
        } else {
            let _ = writeln!(out, "no matches for '{query}'");
        }
    } else if show < total {
        let _ = writeln!(out, "(showing {show} of {total} matches, limit applied)");
    } else if brief {
        let _ = writeln!(out, "{total} match(es)");
    } else {
        let _ = writeln!(out, "{total} matching section(s)");
    }
    Ok(out)
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

/// Split query into lowercase terms for AND matching.
fn query_terms(query: &str) -> Vec<String> {
    query.to_lowercase().split_whitespace().map(String::from).collect()
}

/// Check if ALL terms appear somewhere in the section (AND logic).
fn matches_terms(section: &[&str], terms: &[String]) -> bool {
    if terms.is_empty() { return true; }
    let combined: String = section.iter().map(|l| l.to_lowercase()).collect::<Vec<_>>().join("\n");
    terms.iter().all(|term| combined.contains(term.as_str()))
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

pub fn parse_sections(content: &str) -> Vec<Vec<&str>> {
    let mut sections: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in content.lines() {
        if line.starts_with("## ") && !current.is_empty() {
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
