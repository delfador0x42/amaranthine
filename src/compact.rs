use std::fmt::Write;
use std::fs;
use std::path::Path;

/// Find duplicate/similar entries within a topic and optionally merge them.
pub fn run(dir: &Path, topic: &str, apply: bool) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));
    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = crate::delete::split_sections(&content);
    if sections.len() < 2 {
        return Ok(format!("{filename}: {} entry, nothing to compact", sections.len()));
    }

    // Find similar pairs
    let mut pairs: Vec<(usize, usize, f64)> = Vec::new();
    for i in 0..sections.len() {
        for j in (i + 1)..sections.len() {
            let sim = similarity(sections[i].1, sections[j].1);
            if sim > 0.5 { pairs.push((i, j, sim)); }
        }
    }

    if pairs.is_empty() {
        return Ok(format!("{filename}: {} entries, no duplicates found", sections.len()));
    }

    let mut out = String::new();
    let _ = writeln!(out, "{filename}: {} similar pair(s) found", pairs.len());

    for (i, j, sim) in &pairs {
        let preview_i = entry_preview(sections[*i].1);
        let preview_j = entry_preview(sections[*j].1);
        let _ = writeln!(out, "  [{i}] {preview_i}");
        let _ = writeln!(out, "  [{j}] {preview_j}");
        let _ = writeln!(out, "  similarity: {:.0}%\n", sim * 100.0);
    }

    if !apply {
        let _ = writeln!(out, "run with apply=true to merge (keeps newer, combines bodies)");
        return Ok(out);
    }

    // Merge: for each pair, keep the later entry, combine bodies, remove earlier
    let mut skip: Vec<usize> = Vec::new();
    let mut merges: Vec<(usize, String)> = Vec::new();

    for (i, j, _) in &pairs {
        if skip.contains(i) || skip.contains(j) { continue; }
        // j is always later (higher index = newer)
        let combined = merge_bodies(sections[*i].1, sections[*j].1);
        merges.push((*j, combined));
        skip.push(*i);
    }

    // Rebuild file, skipping merged-away entries and replacing merge targets
    let title = content.lines().next().filter(|l| l.starts_with("# ")).unwrap_or("");
    let mut result = format!("{title}\n");

    for (idx, (hdr, body)) in sections.iter().enumerate() {
        if skip.contains(&idx) { continue; }
        result.push('\n');
        result.push_str(hdr);
        result.push('\n');
        if let Some((_, new_body)) = merges.iter().find(|(mi, _)| *mi == idx) {
            result.push_str(new_body);
        } else {
            let trimmed = body.strip_prefix('\n').unwrap_or(body);
            result.push_str(trimmed);
        }
        if !result.ends_with('\n') { result.push('\n'); }
    }

    crate::config::atomic_write(&filepath, &result)?;
    let new_count = result.matches("\n## ").count();
    let _ = writeln!(out, "compacted: merged {} pairs, {new_count} entries remaining", skip.len());
    Ok(out)
}

/// Scan all topics for compaction opportunities.
pub fn scan(dir: &Path) -> Result<String, String> {
    let files = crate::config::list_topic_files(dir)?;
    let mut out = String::new();
    let mut total_dupes = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();
        let sections = crate::delete::split_sections(&content);
        let mut dupes = 0;
        for i in 0..sections.len() {
            for j in (i + 1)..sections.len() {
                if similarity(sections[i].1, sections[j].1) > 0.5 { dupes += 1; }
            }
        }
        if dupes > 0 {
            let _ = writeln!(out, "  {name}: {dupes} similar pair(s) in {} entries", sections.len());
            total_dupes += dupes;
        }
    }

    if total_dupes == 0 {
        let _ = writeln!(out, "no duplicates found across {} topics", files.len());
    } else {
        let _ = writeln!(out, "\n{total_dupes} total similar pair(s) — use compact <topic> to review");
    }
    Ok(out)
}

/// Word overlap similarity between two text bodies.
fn similarity(a: &str, b: &str) -> f64 {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    let set_a: std::collections::HashSet<&str> = a_lower.split_whitespace()
        .filter(|w| w.len() >= 4).collect();
    let set_b: std::collections::HashSet<&str> = b_lower.split_whitespace()
        .filter(|w| w.len() >= 4).collect();
    if set_a.is_empty() || set_b.is_empty() { return 0.0; }
    let overlap = set_a.intersection(&set_b).count();
    let denom = set_a.len().min(set_b.len());
    overlap as f64 / denom as f64
}

fn entry_preview(body: &str) -> String {
    body.lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with("[tags:"))
        .map(|l| {
            let t = l.trim().trim_start_matches("- ");
            if t.len() > 60 { format!("{}...", &t[..60]) } else { t.to_string() }
        })
        .unwrap_or_else(|| "(empty)".into())
}

fn merge_bodies(older: &str, newer: &str) -> String {
    let newer_trimmed = newer.trim();
    let older_trimmed = older.trim();
    // Combine: newer content first, then unique lines from older
    let newer_lines: Vec<&str> = newer_trimmed.lines().collect();
    let mut result = newer_trimmed.to_string();
    for line in older_trimmed.lines() {
        if line.starts_with("[tags:") { continue; }
        if !newer_lines.iter().any(|n| n.trim() == line.trim()) && !line.trim().is_empty() {
            result.push('\n');
            result.push_str(line);
        }
    }
    result.push('\n');
    result
}
