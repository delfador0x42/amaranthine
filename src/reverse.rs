//! Codebase investigation modes: reverse (architecture map), core (reachability),
//! simplify (similarity + thin wrapper detection). All produce LLM-native output.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

// ── reverse: module-level architecture map ──────────────────────────

pub fn reverse(path: &Path, glob_suffix: &str) -> Result<String, String> {
    if !path.is_dir() { return Err(format!("{} is not a directory", path.display())); }
    let suffix = glob_suffix.trim_start_matches('*');
    let mut fps = Vec::new();
    crate::codepath::walk_files(path, suffix, &mut fps)?;
    fps.sort();

    let mut modules: BTreeMap<String, ModInfo> = BTreeMap::new();
    let mut all_fns: Vec<FnInfo> = Vec::new();
    let mut total_lines = 0usize;

    for fp in &fps {
        let content = match std::fs::read_to_string(fp) { Ok(c) => c, Err(_) => continue };
        let rel = fp.strip_prefix(path).unwrap_or(fp).to_string_lossy().to_string();
        let loc = content.lines().count();
        total_lines += loc;
        let fns = extract_symbols(&content);
        let pub_count = fns.iter().filter(|f| f.is_pub).count();
        all_fns.extend(fns.iter().map(|f| FnInfo {
            name: f.name.clone(), file: rel.clone(), line: f.line,
            end_line: f.end_line, is_pub: f.is_pub,
        }));
        modules.insert(rel, ModInfo { loc, fn_count: fns.len(), pub_count, fns });
    }

    // Cross-module dependency: for each pub fn, find call sites in other files
    let mut in_degree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut out_degree: BTreeMap<&str, usize> = BTreeMap::new();
    for (file, info) in &modules {
        for f in &info.fns {
            if !f.is_pub { continue; }
            for (other_file, other_info) in &modules {
                if other_file == file { continue; }
                // Check if other file calls this function
                for other_fn in &other_info.fns {
                    if other_fn.body_calls.contains(&f.name) {
                        *in_degree.entry(file.as_str()).or_default() += 1;
                        *out_degree.entry(other_file.as_str()).or_default() += 1;
                    }
                }
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "=== ARCHITECTURE: {} ({} files, {}L, {}) ===\n",
        path.display(), modules.len(), total_lines, glob_suffix);

    // Rank modules by centrality (in + out degree)
    let mut ranked: Vec<(&str, usize, usize, usize, usize, usize)> = modules.iter()
        .map(|(name, info)| {
            let i = in_degree.get(name.as_str()).copied().unwrap_or(0);
            let o = out_degree.get(name.as_str()).copied().unwrap_or(0);
            (name.as_str(), i + o, i, o, info.loc, info.pub_count)
        }).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    let _ = writeln!(out, "MODULES (by centrality):");
    for &(name, cent, i, o, loc, pub_c) in &ranked {
        let tag = if cent >= 10 { "[HUB]" }
            else if i >= 5 { "[CORE]" }
            else if cent == 0 { "[EDGE]" }
            else { "" };
        let _ = writeln!(out, "  {name} {tag} {loc}L, {pub_c} pub, in={i} out={o} centrality={cent}");
    }

    // Entry points
    let _ = writeln!(out, "\nENTRY POINTS:");
    for f in &all_fns {
        if f.name == "main" || f.name == "run" {
            let _ = writeln!(out, "  {}:{} — {}", f.file, f.line, f.name);
        }
    }

    // Type hubs: structs/enums/traits mentioned across multiple files
    let _ = writeln!(out, "\nTYPE HUBS:");
    let mut type_refs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (file, info) in &modules {
        for t in &info.fns {
            for dep in &t.body_calls {
                // Heuristic: capitalized names are likely types
                if dep.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    type_refs.entry(dep.clone()).or_default().insert(file.clone());
                }
            }
        }
    }
    let mut type_ranked: Vec<(&str, usize)> = type_refs.iter()
        .filter(|(_, files)| files.len() >= 2)
        .map(|(name, files)| (name.as_str(), files.len()))
        .collect();
    type_ranked.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, count) in type_ranked.iter().take(10) {
        let _ = writeln!(out, "  {name} — used in {count} files");
    }

    let _ = writeln!(out, "\n{} functions, {} files, {}L total",
        all_fns.len(), modules.len(), total_lines);
    Ok(out)
}

// ── core: reachability from entry points ────────────────────────────

pub fn core(path: &Path, glob_suffix: &str, entry_pattern: &str) -> Result<String, String> {
    if !path.is_dir() { return Err(format!("{} is not a directory", path.display())); }
    let suffix = glob_suffix.trim_start_matches('*');
    let mut fps = Vec::new();
    crate::codepath::walk_files(path, suffix, &mut fps)?;
    fps.sort();

    let mut all_fns: Vec<FnInfo> = Vec::new();
    let mut file_contents: Vec<(String, String)> = Vec::new();

    for fp in &fps {
        let content = match std::fs::read_to_string(fp) { Ok(c) => c, Err(_) => continue };
        let rel = fp.strip_prefix(path).unwrap_or(fp).to_string_lossy().to_string();
        let fns = extract_symbols(&content);
        all_fns.extend(fns.iter().map(|f| FnInfo {
            name: f.name.clone(), file: rel.clone(), line: f.line,
            end_line: f.end_line, is_pub: f.is_pub,
        }));
        file_contents.push((rel, content));
    }

    // Build call graph adjacency: fn_name → set of called fn_names
    let mut call_adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let fn_names: BTreeSet<String> = all_fns.iter().map(|f| f.name.clone()).collect();
    for (_, content) in &file_contents {
        for sym in extract_symbols(content) {
            let callees: BTreeSet<String> = sym.body_calls.into_iter()
                .filter(|c| fn_names.contains(c))
                .collect();
            call_adj.entry(sym.name).or_default().extend(callees);
        }
    }

    // Find entry points
    let entry_patterns: Vec<&str> = entry_pattern.split('|').collect();
    let entries: Vec<&FnInfo> = all_fns.iter().filter(|f| {
        entry_patterns.iter().any(|p| {
            let p = p.trim();
            if p.starts_with('#') { false } // skip attribute patterns for now
            else if let Some(name) = p.strip_prefix("fn ") { f.name == name.trim() }
            else { f.name == p }
        })
    }).collect();

    // BFS from entries
    let mut reachable: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = entries.iter().map(|f| f.name.clone()).collect();
    while let Some(name) = queue.pop() {
        if !reachable.insert(name.clone()) { continue; }
        if let Some(callees) = call_adj.get(&name) {
            for c in callees {
                if !reachable.contains(c) { queue.push(c.clone()); }
            }
        }
    }

    // Compute in-degree for ranking
    let mut in_deg: BTreeMap<&str, usize> = BTreeMap::new();
    for callees in call_adj.values() {
        for c in callees {
            if reachable.contains(c.as_str()) {
                *in_deg.entry(c.as_str()).or_default() += 1;
            }
        }
    }

    let dead: Vec<&FnInfo> = all_fns.iter()
        .filter(|f| !reachable.contains(&f.name))
        .collect();

    let mut out = String::new();
    let _ = writeln!(out, "=== CORE: {} ({} entries, {}) ===\n",
        path.display(), entries.len(), glob_suffix);

    let _ = writeln!(out, "ENTRIES ({}):", entries.len());
    for e in &entries {
        let _ = writeln!(out, "  {}:{} — {}", e.file, e.line, e.name);
    }

    let _ = writeln!(out, "\nREACHABLE: {} of {} functions", reachable.len(), all_fns.len());

    // Rank reachable by in-degree
    let _ = writeln!(out, "\nCORE FUNCTIONS (by in-degree):");
    let mut core_ranked: Vec<(&str, usize)> = reachable.iter()
        .map(|n| (n.as_str(), in_deg.get(n.as_str()).copied().unwrap_or(0)))
        .collect();
    core_ranked.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, deg) in core_ranked.iter().take(20) {
        if let Some(f) = all_fns.iter().find(|f| f.name == *name) {
            let _ = writeln!(out, "  {} ({}:{}) in={deg}", name, f.file, f.line);
        }
    }

    let _ = writeln!(out, "\nDEAD CODE: {} functions", dead.len());
    for f in dead.iter().take(30) {
        let _ = writeln!(out, "  {} ({}:{}){}", f.name, f.file, f.line,
            if f.is_pub { " [pub]" } else { "" });
    }
    if dead.len() > 30 {
        let _ = writeln!(out, "  ... +{} more", dead.len() - 30);
    }

    let _ = writeln!(out, "\n{} reachable, {} dead, {} total",
        reachable.len(), dead.len(), all_fns.len());
    Ok(out)
}

// ── simplify: similarity + thin wrapper detection ───────────────────

pub fn simplify(path: &Path, glob_suffix: &str) -> Result<String, String> {
    if !path.is_dir() { return Err(format!("{} is not a directory", path.display())); }
    let suffix = glob_suffix.trim_start_matches('*');
    let mut fps = Vec::new();
    crate::codepath::walk_files(path, suffix, &mut fps)?;
    fps.sort();

    struct FileInfo {
        rel: String,
        loc: usize,
        pub_count: usize,
        fn_count: usize,
        tokens: BTreeSet<String>,
    }

    let mut files: Vec<FileInfo> = Vec::new();
    let mut total_loc = 0usize;

    for fp in &fps {
        let content = match std::fs::read_to_string(fp) { Ok(c) => c, Err(_) => continue };
        let rel = fp.strip_prefix(path).unwrap_or(fp).to_string_lossy().to_string();
        let loc = content.lines().count();
        total_loc += loc;
        let syms = extract_symbols(&content);
        let pub_count = syms.iter().filter(|s| s.is_pub).count();
        // Tokenize file content for similarity
        let tokens: BTreeSet<String> = content.split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|w| w.len() >= 3)
            .map(|w| w.to_lowercase())
            .collect();
        files.push(FileInfo { rel, loc, pub_count, fn_count: syms.len(), tokens });
    }

    let mut out = String::new();
    let _ = writeln!(out, "=== SIMPLIFY: {} ({} files, {}L, {}) ===\n",
        path.display(), files.len(), total_loc, glob_suffix);

    // Cross-file Jaccard similarity
    let _ = writeln!(out, "SIMILAR FILE PAIRS (>40% token overlap):");
    let mut pairs: Vec<(usize, usize, f64)> = Vec::new();
    for i in 0..files.len() {
        // Cap comparisons per file to avoid O(n^2) blowup on large codebases
        let mut pair_count = 0;
        for j in (i + 1)..files.len() {
            if pair_count >= 50 { break; }
            let intersection = files[i].tokens.intersection(&files[j].tokens).count();
            let union = files[i].tokens.len() + files[j].tokens.len() - intersection;
            if union == 0 { continue; }
            let jaccard = intersection as f64 / union as f64;
            if jaccard > 0.40 {
                pairs.push((i, j, jaccard));
            }
            pair_count += 1;
        }
    }
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut consolidation_loc = 0usize;
    if pairs.is_empty() {
        let _ = writeln!(out, "  (none found)");
    } else {
        for &(i, j, jac) in pairs.iter().take(15) {
            let smaller = files[i].loc.min(files[j].loc);
            consolidation_loc += smaller / 2; // conservative estimate
            let _ = writeln!(out, "  {:.0}% — {} ({}L) <> {} ({}L)  SUGGESTION: consolidate (~{}L saved)",
                jac * 100.0, files[i].rel, files[i].loc, files[j].rel, files[j].loc, smaller / 2);
        }
        if pairs.len() > 15 {
            let _ = writeln!(out, "  ... +{} more pairs", pairs.len() - 15);
        }
    }

    // Thin wrappers: files with 0-1 pub symbols
    let _ = writeln!(out, "\nTHIN WRAPPERS (0-1 pub symbols):");
    let mut thin: Vec<&FileInfo> = files.iter()
        .filter(|f| f.pub_count <= 1 && f.loc > 5)
        .collect();
    thin.sort_by(|a, b| a.loc.cmp(&b.loc));
    if thin.is_empty() {
        let _ = writeln!(out, "  (none found)");
    } else {
        for f in thin.iter().take(15) {
            let _ = writeln!(out, "  {} — {}L, {} pub, {} fns",
                f.rel, f.loc, f.pub_count, f.fn_count);
        }
    }

    let _ = writeln!(out, "\nSAVINGS ESTIMATE: ~{}L consolidation potential from {} similar pairs",
        consolidation_loc, pairs.len());
    let _ = writeln!(out, "TOTAL: {} files, {}L, {} thin wrappers",
        files.len(), total_loc, thin.len());
    Ok(out)
}

// ── shared helpers ──────────────────────────────────────────────────

struct SymInfo {
    name: String,
    line: usize,
    end_line: usize,
    is_pub: bool,
    body_calls: BTreeSet<String>,
}

#[allow(dead_code)]
struct FnInfo {
    name: String,
    file: String,
    line: usize,
    end_line: usize,
    is_pub: bool,
}

#[allow(dead_code)]
struct ModInfo {
    loc: usize,
    fn_count: usize,
    pub_count: usize,
    fns: Vec<SymInfo>,
}

fn extract_symbols(content: &str) -> Vec<SymInfo> {
    let lines: Vec<&str> = content.lines().collect();
    let mut syms: Vec<SymInfo> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("//") { continue; }
        if let Some((name, is_pub)) = parse_symbol(t) {
            syms.push(SymInfo {
                name, line: i + 1, end_line: 0, is_pub,
                body_calls: BTreeSet::new(),
            });
        }
    }

    // Set end lines and extract body calls
    for i in 0..syms.len() {
        syms[i].end_line = if i + 1 < syms.len() { syms[i + 1].line - 1 } else { lines.len() };
        let start = syms[i].line; // 1-indexed, body starts after signature
        let end = syms[i].end_line.min(lines.len());
        let mut calls = BTreeSet::new();
        for li in start..end {
            let bytes = lines[li].as_bytes();
            for j in 1..bytes.len() {
                if bytes[j] != b'(' { continue; }
                let mut k = j;
                while k > 0 && (bytes[k - 1].is_ascii_alphanumeric() || bytes[k - 1] == b'_') {
                    k -= 1;
                }
                if j > k + 1 {
                    let name = &lines[li][k..j];
                    if !is_noise(name) { calls.insert(name.to_string()); }
                }
            }
        }
        syms[i].body_calls = calls;
    }

    syms
}

fn parse_symbol(line: &str) -> Option<(String, bool)> {
    let is_pub = line.starts_with("pub ");
    if let Some(idx) = line.find("fn ") {
        if idx > 0 {
            let before = line[..idx].trim();
            if !before.is_empty() && !before.split_whitespace()
                .all(|w| matches!(w, "pub" | "pub(crate)" | "pub(super)" | "async"
                    | "unsafe" | "const" | "extern" | "\"C\"")) {
                return None;
            }
        }
        let rest = &line[idx + 3..];
        let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
        let name = &rest[..end];
        if name.len() >= 2 { return Some((name.to_string(), is_pub)); }
    }
    None
}

fn is_noise(s: &str) -> bool {
    matches!(s, "if" | "for" | "while" | "match" | "return" | "let" | "Some" | "None"
        | "Ok" | "Err" | "Box" | "Vec" | "String" | "format" | "write" | "writeln"
        | "println" | "eprintln" | "assert" | "assert_eq" | "panic" | "todo"
        | "fn" | "pub" | "use" | "mod" | "impl" | "self" | "as" | "in" | "unsafe"
        | "async" | "move" | "type" | "where" | "mut" | "ref" | "true" | "false")
}
