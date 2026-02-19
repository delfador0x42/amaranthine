//! Architecture reconstruction: read matching topics fully + search others.
//! Combines read_topic + search into a single composite query.

use std::fmt::Write;
use std::fs;
use std::path::Path;

pub fn run(dir: &Path, query: &str) -> Result<String, String> {
    let q = crate::config::sanitize_topic(query);
    let files = crate::config::list_topic_files(dir)?;

    let mut primary = Vec::new();
    let mut other = Vec::new();
    for path in &files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if name.contains(&q) {
            primary.push((name, path.clone()));
        } else {
            other.push(name);
        }
    }

    let mut out = String::new();
    if primary.is_empty() {
        let _ = writeln!(out, "No topics matching '{}'. Searching all topics...\n", query);
    } else {
        let _ = writeln!(out, "Architecture: {} ({} primary topics)\n", query, primary.len());
        // Read primary topics fully
        for (name, path) in &primary {
            let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
            let count = content.lines().filter(|l| l.starts_with("## ")).count();
            let _ = writeln!(out, "== {} ({} entries) ==", name, count);
            let _ = writeln!(out, "{}\n", content.trim());
        }
    }

    // Search other topics for related entries
    let filter = crate::search::Filter {
        after: None, before: None, tag: None, topic: None,
        mode: crate::search::SearchMode::Or,
    };
    let related = crate::search::run_medium(dir, query, Some(20), &filter)?;
    // Filter out entries from primary topics (already shown in full)
    let primary_names: Vec<&str> = primary.iter().map(|(n, _)| n.as_str()).collect();
    let filtered: String = related.lines()
        .filter(|line| {
            if let Some(start) = line.find('[') {
                if let Some(end) = line[start..].find(']') {
                    let topic = &line[start+1..start+end];
                    return !primary_names.contains(&topic);
                }
            }
            true
        })
        .collect::<Vec<_>>().join("\n");

    if !filtered.trim().is_empty() {
        let _ = writeln!(out, "== Related entries from other topics ==");
        let _ = writeln!(out, "{}", filtered.trim());
    }
    Ok(out)
}
