//! Live codebase structural analysis: search files for a pattern, categorize access sites.
//! Returns a coupling profile — structured knowledge about how a symbol is used.
//! Zero external deps. Fixed-string search covers 95% of refactoring analysis.

use std::fmt::Write;
use std::path::{Path, PathBuf};

struct Hit {
    file: String,
    line: usize,
    content: String,
    category: &'static str,
}

/// Search `path` for `pattern` in files matching `glob_suffix`, categorize each hit.
pub fn run(pattern: &str, path: &Path, glob_suffix: &str, context: usize)
    -> Result<String, String>
{
    if pattern.is_empty() { return Err("pattern is required".into()); }
    if !path.is_dir() { return Err(format!("{} is not a directory", path.display())); }

    let suffix = glob_suffix.trim_start_matches('*');
    let mut files = Vec::new();
    walk_files(path, suffix, &mut files)?;
    files.sort();

    let mut all_hits: Vec<Hit> = Vec::new();
    for file in &files {
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue, // skip binary/unreadable files
        };
        let rel = file.strip_prefix(path).unwrap_or(file);
        let rel_str = rel.to_string_lossy().to_string();
        for hit in search_file(&content, pattern, &rel_str, context) {
            all_hits.push(hit);
        }
    }

    if all_hits.is_empty() {
        return Ok(format!("no matches for `{pattern}` in {} ({glob_suffix})\n",
            path.display()));
    }

    format_results(&all_hits, pattern, path, glob_suffix)
}

fn walk_files(dir: &Path, suffix: &str, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden dirs and common noise
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            walk_files(&path, suffix, out)?;
        } else if path.to_string_lossy().ends_with(suffix) {
            out.push(path);
        }
    }
    Ok(())
}

fn search_file(content: &str, pattern: &str, rel_path: &str, _context: usize) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        if line.contains(pattern) {
            let trimmed = line.trim();
            // Skip comments
            if trimmed.starts_with("//") || trimmed.starts_with("///") { continue; }
            let category = categorize(line, pattern);
            hits.push(Hit {
                file: rel_path.to_string(),
                line: line_idx + 1,
                content: trimmed.to_string(),
                category,
            });
        }
    }
    hits
}

/// Heuristic categorization by inspecting the match line.
fn categorize(line: &str, pattern: &str) -> &'static str {
    let idx = match line.find(pattern) {
        Some(i) => i,
        None => return "field_access",
    };
    let after = &line[idx + pattern.len()..];
    let after_trimmed = after.trim_start();

    // Clone patterns
    if after_trimmed.starts_with(".clone()")
        || after_trimmed.starts_with(".to_string()")
        || after_trimmed.starts_with(".to_owned()") {
        return "clone";
    }
    // Method call (dot followed by identifier)
    if after_trimmed.starts_with('.') {
        // Check for comparison methods
        if after_trimmed.starts_with(".contains(")
            || after_trimmed.starts_with(".starts_with(")
            || after_trimmed.starts_with(".ends_with(") {
            return "method_call";
        }
        // Map key patterns
        if after_trimmed.starts_with(".entry(")
            || after_trimmed.starts_with(".insert(")
            || after_trimmed.starts_with(".get(") {
            return "map_key";
        }
        return "method_call";
    }
    // Comparison
    if line.contains("==") || line.contains("!=") {
        return "comparison";
    }
    // Format arg
    if line.contains("format!") || line.contains("println!")
        || line.contains("writeln!") || line.contains("write!")
        || line.contains("\"{") {
        return "format_arg";
    }
    // Map key via index/entry before the pattern
    let before = &line[..idx];
    if before.contains(".entry(") || before.contains(".insert(")
        || before.trim_end().ends_with('[') {
        return "map_key";
    }
    "field_access"
}

fn format_results(hits: &[Hit], pattern: &str, path: &Path, glob: &str)
    -> Result<String, String>
{
    let mut out = String::new();
    let _ = writeln!(out, "# codepath: `{pattern}` in {} ({glob})\n",
        path.display());

    // Group by file
    let mut current_file = "";
    let mut file_count = 0usize;
    let mut cats: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();

    // Count per file for headers
    let mut file_counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for h in hits { *file_counts.entry(&h.file).or_insert(0) += 1; }

    for h in hits {
        if h.file != current_file {
            if !current_file.is_empty() { let _ = writeln!(out); }
            current_file = &h.file;
            file_count += 1;
            let fc = file_counts.get(h.file.as_str()).unwrap_or(&0);
            let _ = writeln!(out, "## {} ({fc} sites)", h.file);
        }
        let short = truncate_line(&h.content, 70);
        let _ = writeln!(out, "  L{:<4} {:70} → {}", h.line, short, h.category);
        *cats.entry(h.category).or_insert(0) += 1;
    }

    let _ = writeln!(out, "\n## Summary");
    let _ = writeln!(out, "{} sites across {file_count} files", hits.len());
    for (cat, count) in &cats {
        let _ = writeln!(out, "  {cat:<16} {count}");
    }
    Ok(out)
}

fn truncate_line(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) { end -= 1; }
        format!("{}...", &s[..end])
    }
}
