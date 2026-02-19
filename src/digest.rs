use std::fmt::Write;
use std::path::Path;

pub fn run(dir: &Path) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Ok("no data.log found\n".into()); }
    crate::cache::with_corpus(dir, |cached| {
        if cached.is_empty() { return "no entries\n".into(); }
        // Group by topic preserving order
        let mut topic_order: Vec<String> = Vec::new();
        let mut grouped: std::collections::BTreeMap<&str, Vec<&crate::cache::CachedEntry>> =
            std::collections::BTreeMap::new();
        for e in cached {
            if !grouped.contains_key(e.topic.as_str()) { topic_order.push(e.topic.to_string()); }
            grouped.entry(e.topic.as_str()).or_default().push(e);
        }
        let mut out = String::new();
        for (i, name) in topic_order.iter().enumerate() {
            let group = &grouped[name.as_str()];
            if i > 0 { let _ = writeln!(out); }
            let latest = group.last().map(|e| e.date_str())
                .unwrap_or_else(|| "empty".into());
            let _ = writeln!(out, "### {} ({} entries, last: {})", name, group.len(), latest);
            for e in group {
                let preview = e.preview();
                let preview = if preview.is_empty() { "(empty)" } else { preview };
                let _ = writeln!(out, "- {}", crate::text::truncate(preview.trim().trim_start_matches("- "), 100));
            }
        }
        out
    })
}
