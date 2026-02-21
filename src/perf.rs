//! Performance trace: callgraph from entry point with antipattern annotations.
//! Flags allocations, clones, syscalls, locks, and formatting in hot paths.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

struct PerfFn {
    name: String,
    file: String,
    line: usize,
    antipatterns: Vec<(usize, &'static str, &'static str)>, // (line, category, detail)
}

pub fn run(path: &Path, glob_suffix: &str, entry: &str, depth: usize) -> Result<String, String> {
    if entry.is_empty() { return Err("entry function name is required".into()); }
    if !path.is_dir() { return Err(format!("{} is not a directory", path.display())); }

    let suffix = glob_suffix.trim_start_matches('*');
    let mut fps = Vec::new();
    crate::codepath::walk_files(path, suffix, &mut fps)?;
    fps.sort();

    // Extract all function definitions and their bodies
    let mut all_fns: BTreeMap<String, (String, usize, usize)> = BTreeMap::new(); // name → (file, start, end)
    let mut files: Vec<(String, Vec<String>)> = Vec::new();

    for fp in &fps {
        let content = match std::fs::read_to_string(fp) { Ok(c) => c, Err(_) => continue };
        let rel = fp.strip_prefix(path).unwrap_or(fp).to_string_lossy().to_string();
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let fns = extract_fns(&lines);
        for (name, start, end) in &fns {
            all_fns.insert(name.clone(), (rel.clone(), *start, *end));
        }
        files.push((rel, lines));
    }

    // BFS from entry through call sites
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<(String, usize)> = vec![(entry.to_string(), 0)];
    let mut chain: Vec<PerfFn> = Vec::new();

    while let Some((name, d)) = queue.pop() {
        if d > depth.min(5) { continue; }
        if !visited.insert(name.clone()) { continue; }

        let (file, start, end) = match all_fns.get(&name) {
            Some(v) => v.clone(),
            None => continue,
        };

        // Get file lines
        let lines = match files.iter().find(|(f, _)| *f == file) {
            Some((_, l)) => l,
            None => continue,
        };

        // Scan function body for antipatterns
        let mut antipatterns = Vec::new();
        let body_start = start.saturating_sub(1);
        let body_end = end.min(lines.len());
        let mut callees = Vec::new();

        for li in body_start..body_end {
            let t = lines[li].trim();
            if t.starts_with("//") { continue; }

            // Detect antipatterns
            for (pat, cat, detail) in PATTERNS {
                if t.contains(pat) {
                    antipatterns.push((li + 1, *cat, *detail));
                }
            }

            // Collect callees for BFS
            let bytes = lines[li].as_bytes();
            for j in 1..bytes.len() {
                if bytes[j] != b'(' { continue; }
                let mut k = j;
                while k > 0 && (bytes[k - 1].is_ascii_alphanumeric() || bytes[k - 1] == b'_') {
                    k -= 1;
                }
                if j > k + 1 {
                    let callee = &lines[li][k..j];
                    if !is_noise(callee) && all_fns.contains_key(callee) {
                        callees.push(callee.to_string());
                    }
                }
            }
        }

        for c in callees {
            if !visited.contains(&c) {
                queue.push((c, d + 1));
            }
        }

        chain.push(PerfFn { name, file, line: start, antipatterns });
    }

    // Output
    let mut out = String::new();
    let _ = writeln!(out, "=== PERF: {}() depth={} in {} ({}) ===\n",
        entry, depth, path.display(), glob_suffix);

    if chain.is_empty() {
        let _ = writeln!(out, "Function '{}' not found in codebase.", entry);
        return Ok(out);
    }

    let _ = writeln!(out, "PATH ({} functions reachable):", chain.len());
    let mut total_issues = 0usize;
    let mut cat_counts: BTreeMap<&str, usize> = BTreeMap::new();

    for pf in &chain {
        let tag = if pf.antipatterns.is_empty() { "[CLEAN]" }
            else {
                let cats: BTreeSet<&str> = pf.antipatterns.iter().map(|(_, c, _)| *c).collect();
                if cats.contains("ALLOC") || cats.contains("FORMAT") { "[ALLOC]" }
                else if cats.contains("SYSCALL") { "[SYSCALL]" }
                else if cats.contains("LOCK") { "[LOCK]" }
                else if cats.contains("CLONE") { "[CLONE]" }
                else { "[WARN]" }
            };
        let _ = writeln!(out, "  {} {} ({}:{})", tag, pf.name, pf.file, pf.line);
        for (line, cat, detail) in &pf.antipatterns {
            let _ = writeln!(out, "    L{} [{}] {}", line, cat, detail);
            *cat_counts.entry(cat).or_default() += 1;
            total_issues += 1;
        }
    }

    let _ = writeln!(out, "\nANTIPATTERNS: {} issues across {} functions", total_issues, chain.len());
    for (cat, count) in &cat_counts {
        let severity = match *cat {
            "ALLOC" | "FORMAT" => "per-call cost",
            "CLONE" => "avoidable copy",
            "SYSCALL" => "kernel boundary",
            "LOCK" => "contention risk",
            _ => "overhead",
        };
        let _ = writeln!(out, "  {} — {} ({severity})", cat, count);
    }

    let clean = chain.iter().filter(|f| f.antipatterns.is_empty()).count();
    let _ = writeln!(out, "\nSUMMARY: {} clean, {} flagged of {} reachable functions",
        clean, chain.len() - clean, chain.len());
    Ok(out)
}

const PATTERNS: &[(&str, &str, &str)] = &[
    // Allocation
    ("Vec::new", "ALLOC", "Vec heap allocation"),
    ("Vec::with_capacity", "ALLOC", "Vec heap allocation (pre-sized)"),
    ("String::new", "ALLOC", "String heap allocation"),
    ("String::with_capacity", "ALLOC", "String heap allocation (pre-sized)"),
    (".to_string()", "ALLOC", "String conversion/allocation"),
    (".to_vec()", "ALLOC", "Vec copy/allocation"),
    ("Box::new", "ALLOC", "Box heap allocation"),
    ("format!", "FORMAT", "format macro — heap alloc + formatting"),
    // Clone
    (".clone()", "CLONE", "explicit clone"),
    (".to_owned()", "CLONE", "to_owned clone"),
    // Syscall
    ("std::fs::read", "SYSCALL", "filesystem read"),
    ("std::fs::write", "SYSCALL", "filesystem write"),
    ("fs::read", "SYSCALL", "filesystem read"),
    ("fs::write", "SYSCALL", "filesystem write"),
    ("read_to_string", "SYSCALL", "read entire file to String"),
    ("read_dir", "SYSCALL", "directory listing"),
    ("OpenOptions", "SYSCALL", "file open"),
    // Lock
    (".lock()", "LOCK", "mutex lock acquisition"),
    (".read()", "LOCK", "rwlock read acquisition"),
    (".write()", "LOCK", "rwlock write acquisition"),
    // Format (output)
    ("println!", "FORMAT", "println — stdout + formatting"),
    ("eprintln!", "FORMAT", "eprintln — stderr + formatting"),
    ("writeln!", "FORMAT", "writeln — formatting into buffer"),
    ("write!", "FORMAT", "write — formatting into buffer"),
];

fn extract_fns(lines: &[String]) -> Vec<(String, usize, usize)> {
    let mut fns: Vec<(String, usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("//") { continue; }
        if let Some(name) = parse_fn_name(t) {
            fns.push((name, i + 1, 0));
        }
    }
    for i in 0..fns.len() {
        fns[i].2 = if i + 1 < fns.len() { fns[i + 1].1 - 1 } else { lines.len() };
    }
    fns
}

fn parse_fn_name(line: &str) -> Option<String> {
    let idx = line.find("fn ")?;
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
    if name.len() >= 2 { Some(name.to_string()) } else { None }
}

fn is_noise(s: &str) -> bool {
    matches!(s, "if" | "for" | "while" | "match" | "return" | "let" | "Some" | "None"
        | "Ok" | "Err" | "Box" | "Vec" | "String" | "format" | "write" | "writeln"
        | "println" | "eprintln" | "assert" | "assert_eq" | "panic" | "todo"
        | "fn" | "pub" | "use" | "mod" | "impl" | "self" | "as" | "in" | "unsafe"
        | "async" | "move" | "type" | "where" | "mut" | "ref" | "true" | "false")
}
