//! Topic dependency graph from binary index xrefs (near-instant).
//! Falls back to corpus scan if index unavailable.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

pub fn run(dir: &Path) -> Result<String, String> {
    run_filtered(dir, None)
}

pub fn run_focused(dir: &Path, focus: &str) -> Result<String, String> {
    run_filtered(dir, Some(focus))
}

fn run_filtered(dir: &Path, focus: Option<&str>) -> Result<String, String> {
    // Try index path first (pre-computed xrefs)
    if let Some(result) = run_via_index(dir, focus) { return Ok(result); }
    // Fallback: corpus scan with token_set matching
    run_via_corpus(dir, focus)
}

fn matches_focus(name: &str, focus: Option<&str>) -> bool {
    match focus {
        None => true,
        Some(pat) => glob_match(pat, name),
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 { return text.contains(pattern); }
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() { continue; }
        if i == 0 {
            if !text.starts_with(part) { return false; }
            pos = part.len();
        } else if i == parts.len() - 1 {
            if !text[pos..].ends_with(part) { return false; }
        } else {
            match text[pos..].find(part) {
                Some(idx) => pos += idx + part.len(),
                None => return false,
            }
        }
    }
    true
}

fn run_via_index(dir: &Path, focus: Option<&str>) -> Option<String> {
    crate::mcp::ensure_index_fresh(dir);
    crate::mcp::with_index(|data| {
        let topics = crate::binquery::topic_table(data).ok()?;
        let xrefs = crate::binquery::xref_edges(data).ok()?;

        let mut outgoing: BTreeMap<u16, BTreeMap<u16, usize>> = BTreeMap::new();
        let mut incoming: BTreeMap<u16, BTreeMap<u16, usize>> = BTreeMap::new();
        for (src, dst, count) in &xrefs {
            *outgoing.entry(*src).or_default().entry(*dst).or_insert(0) += *count as usize;
            *incoming.entry(*dst).or_default().entry(*src).or_insert(0) += *count as usize;
        }

        let name_of = |id: u16| -> &str {
            topics.iter().find(|(i, _, _)| *i == id).map(|(_, n, _)| n.as_str()).unwrap_or("?")
        };

        let mut sorted: Vec<(u16, usize)> = topics.iter()
            .filter(|(_, name, _)| matches_focus(name, focus))
            .map(|(id, _, _)| {
                let oc: usize = outgoing.get(id).map(|m| m.values().sum()).unwrap_or(0);
                let ic: usize = incoming.get(id).map(|m| m.values().sum()).unwrap_or(0);
                (*id, oc + ic)
            }).collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        let connected = sorted.iter().filter(|(_, c)| *c > 0).count();
        let mut out = String::new();
        let focus_label = focus.map(|f| format!(" (focus: {f})")).unwrap_or_default();
        let _ = writeln!(out, "Topic dependency graph ({} topics, {} edges, {} connected{}):\n",
            sorted.len(), xrefs.len(), connected, focus_label);

        for (id, total) in &sorted {
            if *total == 0 { continue; }
            let _ = writeln!(out, "{} ({} refs)", name_of(*id), total);
            if let Some(targets) = outgoing.get(id) {
                let mut refs: Vec<(u16, usize)> = targets.iter().map(|(k, v)| (*k, *v)).collect();
                refs.sort_by(|a, b| b.1.cmp(&a.1));
                let items: Vec<String> = refs.iter().take(8)
                    .map(|(t, c)| format!("{}({})", name_of(*t), c)).collect();
                let _ = writeln!(out, "  -> {}", items.join(" "));
            }
            if let Some(sources) = incoming.get(id) {
                let mut refs: Vec<(u16, usize)> = sources.iter().map(|(k, v)| (*k, *v)).collect();
                refs.sort_by(|a, b| b.1.cmp(&a.1));
                let items: Vec<String> = refs.iter().take(8)
                    .map(|(t, c)| format!("{}({})", name_of(*t), c)).collect();
                let _ = writeln!(out, "  <- {}", items.join(" "));
            }
            let _ = writeln!(out);
        }
        Some(out)
    }).flatten()
}

fn run_via_corpus(dir: &Path, focus: Option<&str>) -> Result<String, String> {
    crate::cache::with_corpus(dir, |entries| {
        let mut names_set = std::collections::BTreeSet::new();
        for e in entries { names_set.insert(e.topic.as_str()); }
        let names: Vec<&str> = names_set.into_iter().collect();

        let mut outgoing: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
        let mut incoming: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();

        for e in entries {
            for target in &names {
                if *target == e.topic.as_str() { continue; }
                // Use tf_map for matching instead of body.to_lowercase()
                let target_tokens = crate::text::tokenize(target);
                let all_match = target_tokens.iter()
                    .filter(|t| t.len() >= 2)
                    .all(|t| e.tf_map.contains_key(t));
                if all_match && !target_tokens.is_empty() {
                    *outgoing.entry(e.topic.as_str()).or_default()
                        .entry(target).or_insert(0) += 1;
                    *incoming.entry(target).or_default()
                        .entry(e.topic.as_str()).or_insert(0) += 1;
                }
            }
        }

        let mut topics: Vec<(&str, usize)> = names.iter()
            .filter(|n| matches_focus(n, focus))
            .map(|n| {
                let oc: usize = outgoing.get(n).map(|m| m.values().sum()).unwrap_or(0);
                let ic: usize = incoming.get(n).map(|m| m.values().sum()).unwrap_or(0);
                (*n, oc + ic)
            }).collect();
        topics.sort_by(|a, b| b.1.cmp(&a.1));

        let total_edges: usize = outgoing.values().map(|m| m.len()).sum();
        let connected = topics.iter().filter(|(_, c)| *c > 0).count();
        let mut out = String::new();
        let focus_label = focus.map(|f| format!(" (focus: {f})")).unwrap_or_default();
        let _ = writeln!(out, "Topic dependency graph ({} topics, {} edges, {} connected{}):\n",
            topics.len(), total_edges, connected, focus_label);

        for (name, total) in &topics {
            if *total == 0 { continue; }
            let _ = writeln!(out, "{} ({} refs)", name, total);
            if let Some(targets) = outgoing.get(name) {
                let mut refs: Vec<(&&str, &usize)> = targets.iter().collect();
                refs.sort_by(|a, b| b.1.cmp(a.1));
                let items: Vec<String> = refs.iter().take(8)
                    .map(|(t, c)| format!("{}({})", t, c)).collect();
                let _ = writeln!(out, "  -> {}", items.join(" "));
            }
            if let Some(sources) = incoming.get(name) {
                let mut refs: Vec<(&&str, &usize)> = sources.iter().collect();
                refs.sort_by(|a, b| b.1.cmp(a.1));
                let items: Vec<String> = refs.iter().take(8)
                    .map(|(t, c)| format!("{}({})", t, c)).collect();
                let _ = writeln!(out, "  <- {}", items.join(" "));
            }
            let _ = writeln!(out);
        }
        out
    })
}
