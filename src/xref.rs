use std::fmt::Write;
use std::path::Path;

/// Find all cross-references: entries in other topics that mention this topic.
/// Uses binary index xref edges when available (~1ms), falls back to corpus scan.
pub fn refs_for(dir: &Path, topic: &str) -> Result<String, String> {
    let filename = crate::config::sanitize_topic(topic);

    // Try index path first (pre-computed xref edges)
    if let Some(result) = refs_via_index(dir, &filename) {
        return Ok(result);
    }
    // Fallback: corpus scan with token_set matching
    refs_via_corpus(dir, &filename)
}

fn refs_via_index(dir: &Path, filename: &str) -> Option<String> {
    crate::mcp::ensure_index_fresh(dir);
    crate::mcp::with_index(|data| {
        let topics = crate::binquery::topic_table(data).ok()?;
        let xrefs = crate::binquery::xref_edges(data).ok()?;

        // Find topic ID for the target
        let target_id = topics.iter().find(|(_, name, _)| name == filename).map(|(id, _, _)| *id)?;

        // Verify topic exists
        let name_of = |id: u16| -> &str {
            topics.iter().find(|(i, _, _)| *i == id).map(|(_, n, _)| n.as_str()).unwrap_or("?")
        };

        let mut out = String::new();
        let _ = writeln!(out, "Cross-references for '{filename}':\n");
        let mut total = 0;

        // Find edges where target is the destination (incoming refs = mentions of this topic)
        let mut refs: Vec<(u16, usize)> = Vec::new();
        for (src, dst, count) in &xrefs {
            if *dst == target_id && *src != target_id {
                refs.push((*src, *count as usize));
            }
        }
        refs.sort_by(|a, b| b.1.cmp(&a.1));

        for (src_id, count) in &refs {
            let _ = writeln!(out, "  [{}] ({} mentions)", name_of(*src_id), count);
            total += *count;
        }

        if total == 0 {
            let _ = writeln!(out, "  (no references found in other topics)");
        } else {
            let _ = writeln!(out, "\n{total} reference(s) across {} topics", refs.len());
        }
        Some(out)
    }).flatten()
}

fn refs_via_corpus(dir: &Path, filename: &str) -> Result<String, String> {
    crate::cache::with_corpus(dir, |cached| {
        if !cached.iter().any(|e| e.topic == filename) {
            return Err(format!("topic '{}' not found", filename));
        }

        let search_tokens = crate::text::tokenize(filename);
        let search_tokens: Vec<&str> = search_tokens.iter()
            .filter(|t| t.len() >= 2).map(|s| s.as_str()).collect();

        let mut out = String::new();
        let _ = writeln!(out, "Cross-references for '{filename}':\n");
        let mut total = 0;

        for e in cached {
            if e.topic == filename { continue; }
            // Check if all tokens of the topic name appear in this entry's tf_map
            let all_match = !search_tokens.is_empty()
                && search_tokens.iter().all(|t| e.tf_map.contains_key(*t));
            if all_match {
                let preview = e.body.lines()
                    .find(|l| !crate::text::is_metadata_line(l) && !l.trim().is_empty())
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
    })?
}
