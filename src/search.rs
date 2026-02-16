use std::fmt::Write;
use std::fs;
use std::path::Path;

pub fn run(dir: &Path, query: &str, plain: bool, limit: Option<usize>) -> Result<String, String> {
    search(dir, query, plain, false, limit)
}

pub fn run_brief(dir: &Path, query: &str, limit: Option<usize>) -> Result<String, String> {
    search(dir, query, true, true, limit)
}

pub fn run_topics(dir: &Path, query: &str) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }
    let query_lower = query.to_lowercase();
    let files = crate::config::list_search_files(dir)?;
    let mut hits: Vec<(String, usize)> = Vec::new();
    let mut total = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let sections = parse_sections(&content);
        let mut n = 0;
        for section in &sections {
            if section.iter().any(|l| l.to_lowercase().contains(&query_lower)) {
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

pub fn count(dir: &Path, query: &str) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }
    let query_lower = query.to_lowercase();
    let files = crate::config::list_search_files(dir)?;
    let mut total = 0;
    let mut topics = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let sections = parse_sections(&content);
        let mut file_hits = 0;
        for section in &sections {
            if section.iter().any(|l| l.to_lowercase().contains(&query_lower)) {
                file_hits += 1;
                total += 1;
            }
        }
        if file_hits > 0 { topics += 1; }
    }
    Ok(format!("{total} matches across {topics} topics for '{query}'"))
}

fn search(dir: &Path, query: &str, plain: bool, brief: bool, limit: Option<usize>) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let query_lower = query.to_lowercase();
    let files = crate::config::list_search_files(dir)?;
    let mut total = 0;
    let mut out = String::new();
    let mut limited = false;

    'outer: for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();
        let sections = parse_sections(&content);
        let mut file_matches = 0;

        for section in &sections {
            if section.iter().any(|l| l.to_lowercase().contains(&query_lower)) {
                if brief {
                    if let Some(hit) = section.iter().find(|l| l.to_lowercase().contains(&query_lower)) {
                        let trimmed = hit.trim_start_matches("- ").trim();
                        let short = truncate(trimmed, 80);
                        let _ = writeln!(out, "  [{name}] {short}");
                    }
                } else {
                    if file_matches == 0 {
                        if plain {
                            let _ = writeln!(out, "\n--- {name}.md ---");
                        } else {
                            let _ = writeln!(out, "\n\x1b[1;36m--- {name}.md ---\x1b[0m");
                        }
                    }
                    for line in section {
                        if line.to_lowercase().contains(&query_lower) {
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
                file_matches += 1;
                total += 1;
                if let Some(lim) = limit {
                    if total >= lim { limited = true; break 'outer; }
                }
            }
        }
    }

    if total == 0 {
        let _ = writeln!(out, "no matches for '{query}'");
    } else if limited {
        let _ = writeln!(out, "(showing {total} of {total}+ matches, limit applied)");
    } else if brief {
        let _ = writeln!(out, "{total} match(es)");
    } else {
        let _ = writeln!(out, "{total} matching section(s)");
    }
    Ok(out)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

fn parse_sections(content: &str) -> Vec<Vec<&str>> {
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
