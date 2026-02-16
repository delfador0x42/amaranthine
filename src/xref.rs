use std::fmt::Write;
use std::fs;
use std::path::Path;

/// Find all cross-references: entries in other topics that mention this topic.
pub fn refs_for(dir: &Path, topic: &str) -> Result<String, String> {
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));
    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let files = crate::config::list_topic_files(dir)?;
    let search_terms: Vec<String> = filename.split('-')
        .filter(|p| p.len() >= 3)
        .map(|p| p.to_lowercase())
        .collect();

    let mut out = String::new();
    let _ = writeln!(out, "Cross-references for '{filename}':\n");
    let mut total = 0;

    for path in &files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if name == filename { continue; }

        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let sections = crate::search::parse_sections(&content);

        for section in &sections {
            let combined: String = section.iter()
                .map(|l| l.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");

            // Must match the full topic name OR all significant parts
            let full_match = combined.contains(&filename);
            let part_match = search_terms.len() >= 2
                && search_terms.iter().all(|t| combined.contains(t.as_str()));

            if full_match || part_match {
                let preview = section.iter()
                    .find(|l| !l.starts_with("## ") && !l.starts_with("[tags:") && !l.trim().is_empty())
                    .map(|l| {
                        let t = l.trim();
                        if t.len() > 70 { &t[..70] } else { t }
                    })
                    .unwrap_or("(empty)");
                let _ = writeln!(out, "  [{name}] {preview}");
                total += 1;
            }
        }
    }

    if total == 0 {
        let _ = writeln!(out, "  (no references found in other topics)");
    } else {
        let _ = writeln!(out, "\n{total} reference(s) across other topics");
    }
    Ok(out)
}
