//! Topic dependency graph: scan all topics for cross-references,
//! build bidirectional adjacency list.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::Path;

pub fn run(dir: &Path) -> Result<String, String> {
    let files = crate::config::list_topic_files(dir)?;

    // Collect all topic names
    let names: Vec<String> = files.iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();

    // outgoing[A] = set of topics mentioned IN A's content
    let mut outgoing: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    // incoming[B] = set of topics that mention B
    let mut incoming: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();

    for path in &files {
        let src = path.file_stem().unwrap().to_string_lossy().to_string();
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let content_lower = content.to_lowercase();

        for target in &names {
            if target == &src { continue; }
            // Count mentions of target topic name in source content
            let count = content_lower.matches(target.as_str()).count();
            if count > 0 {
                *outgoing.entry(src.clone()).or_default()
                    .entry(target.clone()).or_insert(0) += count;
                *incoming.entry(target.clone()).or_default()
                    .entry(src.clone()).or_insert(0) += count;
            }
        }
    }

    // Sort by total references (outgoing + incoming)
    let mut topics: Vec<(String, usize)> = names.iter().map(|n| {
        let out_count: usize = outgoing.get(n).map(|m| m.values().sum()).unwrap_or(0);
        let in_count: usize = incoming.get(n).map(|m| m.values().sum()).unwrap_or(0);
        (n.clone(), out_count + in_count)
    }).collect();
    topics.sort_by(|a, b| b.1.cmp(&a.1));

    let total_edges: usize = outgoing.values().map(|m| m.len()).sum();
    let connected = topics.iter().filter(|(_, c)| *c > 0).count();
    let mut out = String::new();
    let _ = writeln!(out, "Topic dependency graph ({} topics, {} edges, {} connected):\n",
        names.len(), total_edges, connected);

    for (name, total) in &topics {
        if *total == 0 { continue; }
        let _ = writeln!(out, "{} ({} refs)", name, total);
        if let Some(targets) = outgoing.get(name) {
            let mut refs: Vec<(&String, &usize)> = targets.iter().collect();
            refs.sort_by(|a, b| b.1.cmp(a.1));
            let items: Vec<String> = refs.iter().take(8)
                .map(|(t, c)| format!("{}({})", t, c)).collect();
            let _ = writeln!(out, "  -> {}", items.join(" "));
        }
        if let Some(sources) = incoming.get(name) {
            let mut refs: Vec<(&String, &usize)> = sources.iter().collect();
            refs.sort_by(|a, b| b.1.cmp(a.1));
            let items: Vec<String> = refs.iter().take(8)
                .map(|(t, c)| format!("{}({})", t, c)).collect();
            let _ = writeln!(out, "  <- {}", items.join(" "));
        }
        let _ = writeln!(out);
    }
    Ok(out)
}
