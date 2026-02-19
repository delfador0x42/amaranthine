use std::fmt::Write;
use std::path::Path;

/// Find all cross-references: entries in other topics that mention this topic.
pub fn refs_for(dir: &Path, topic: &str) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    let entries = crate::datalog::iter_live(&log_path)?;
    let filename = crate::config::sanitize_topic(topic);

    if !entries.iter().any(|e| e.topic == filename) {
        return Err(format!("topic '{}' not found", filename));
    }

    let search_terms: Vec<String> = filename.split('-')
        .filter(|p| p.len() >= 3)
        .map(|p| p.to_lowercase())
        .collect();

    let mut out = String::new();
    let _ = writeln!(out, "Cross-references for '{filename}':\n");
    let mut total = 0;

    for e in &entries {
        if e.topic == filename { continue; }
        let lower = e.body.to_lowercase();
        let full_match = lower.contains(&filename);
        let part_match = search_terms.len() >= 2
            && search_terms.iter().all(|t| lower.contains(t.as_str()));
        if full_match || part_match {
            let preview = e.body.lines()
                .find(|l| !l.starts_with("[tags:") && !l.starts_with("[source:") && !l.trim().is_empty())
                .map(|l| { let t = l.trim(); if t.len() > 70 { &t[..70] } else { t } })
                .unwrap_or("(empty)");
            let _ = writeln!(out, "  [{}] {preview}", e.topic);
            total += 1;
        }
    }

    if total == 0 {
        let _ = writeln!(out, "  (no references found in other topics)");
    } else {
        let _ = writeln!(out, "\n{total} reference(s) across other topics");
    }
    Ok(out)
}
