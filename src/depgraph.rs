//! Topic dependency graph from data.log: scan for cross-references.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

pub fn run(dir: &Path) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    let entries = crate::datalog::iter_live(&log_path)?;

    let mut names_set = std::collections::BTreeSet::new();
    for e in &entries { names_set.insert(e.topic.clone()); }
    let names: Vec<String> = names_set.into_iter().collect();

    let mut outgoing: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut incoming: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();

    for e in &entries {
        let content_lower = e.body.to_lowercase();
        for target in &names {
            if target == &e.topic { continue; }
            let count = content_lower.matches(target.as_str()).count();
            if count > 0 {
                *outgoing.entry(e.topic.clone()).or_default()
                    .entry(target.clone()).or_insert(0) += count;
                *incoming.entry(target.clone()).or_default()
                    .entry(e.topic.clone()).or_insert(0) += count;
            }
        }
    }

    let mut topics: Vec<(String, usize)> = names.iter().map(|n| {
        let oc: usize = outgoing.get(n).map(|m| m.values().sum()).unwrap_or(0);
        let ic: usize = incoming.get(n).map(|m| m.values().sum()).unwrap_or(0);
        (n.clone(), oc + ic)
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
