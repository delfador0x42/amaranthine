//! Callgraph: trace callers and callees of a function across a codebase.
//! Reuses codepath::walk_files for file discovery. Output: call chain tree.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

struct FnDef { name: String, file: String, line: usize, end_line: usize }
struct CallRef { caller: String, file: String, line: usize, snippet: String }

pub fn run(pattern: &str, path: &Path, glob_suffix: &str, depth: usize, direction: &str)
    -> Result<String, String>
{
    if pattern.is_empty() { return Err("pattern is required".into()); }
    if !path.is_dir() { return Err(format!("{} is not a directory", path.display())); }
    let suffix = glob_suffix.trim_start_matches('*');
    let mut fps = Vec::new();
    crate::codepath::walk_files(path, suffix, &mut fps)?;
    fps.sort();

    let mut all_fns: Vec<FnDef> = Vec::new();
    let mut files: Vec<(String, String)> = Vec::new();
    for fp in &fps {
        let content = match std::fs::read_to_string(fp) { Ok(c) => c, Err(_) => continue };
        let rel = fp.strip_prefix(path).unwrap_or(fp).to_string_lossy().to_string();
        for (name, line, end) in extract_fns(&content) {
            all_fns.push(FnDef { name, file: rel.clone(), line, end_line: end });
        }
        files.push((rel, content));
    }

    let mut out = String::new();
    let _ = writeln!(out, "# callgraph: `{}` in {} ({})\n", pattern, path.display(), glob_suffix);

    for d in all_fns.iter().filter(|f| f.name == pattern) {
        let _ = writeln!(out, "DEF: {} ({}:{})", d.name, d.file, d.line);
    }

    if direction != "callees" {
        let _ = writeln!(out, "\nCALLERS:");
        let mut targets = vec![pattern.to_string()];
        let mut seen = BTreeSet::new();
        seen.insert(pattern.to_string());
        for d in 0..depth.min(3) {
            let refs = find_callers(&targets, &files, &all_fns, &seen);
            if refs.is_empty() { break; }
            let indent = "  ".repeat(d + 1);
            let mut next = Vec::new();
            for r in &refs {
                let snip = crate::text::truncate(&r.snippet, 55);
                let _ = writeln!(out, "{}\u{2190} {} ({}:{})  {}", indent, r.caller, r.file, r.line, snip);
                if seen.insert(r.caller.clone()) { next.push(r.caller.clone()); }
            }
            targets = next;
        }
    }

    if direction != "callers" {
        let _ = writeln!(out, "\nCALLEES:");
        for def in all_fns.iter().filter(|f| f.name == pattern) {
            for (name, line) in callees_in_body(def, &files) {
                let _ = writeln!(out, "  \u{2192} {} ({}:{})", name, def.file, line);
            }
        }
    }

    let _ = writeln!(out, "\n{} functions across {} files", all_fns.len(), files.len());
    Ok(out)
}

fn extract_fns(content: &str) -> Vec<(String, usize, usize)> {
    let lines: Vec<&str> = content.lines().collect();
    let mut fns: Vec<(String, usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("//") { continue; }
        if let Some(name) = parse_fn_name(t) { fns.push((name, i + 1, 0)); }
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

fn find_callers(targets: &[String], files: &[(String, String)],
                all_fns: &[FnDef], seen: &BTreeSet<String>) -> Vec<CallRef> {
    let mut refs = Vec::new();
    let mut dedup: BTreeSet<String> = BTreeSet::new();
    for (rel, content) in files {
        let file_fns: Vec<&FnDef> = all_fns.iter().filter(|f| f.file == *rel).collect();
        for (i, line) in content.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("//") { continue; }
            for target in targets {
                if !has_call(t, target) { continue; }
                if parse_fn_name(t).as_deref() == Some(target.as_str()) { continue; }
                let line_no = i + 1;
                let caller = file_fns.iter()
                    .filter(|f| f.line <= line_no && f.end_line >= line_no)
                    .last().map(|f| f.name.as_str()).unwrap_or("<module>");
                if seen.contains(caller) { continue; }
                let key = format!("{}:{}", caller, rel);
                if !dedup.insert(key) { continue; }
                refs.push(CallRef {
                    caller: caller.to_string(), file: rel.clone(),
                    line: line_no, snippet: t.to_string(),
                });
            }
        }
    }
    refs
}

fn has_call(line: &str, target: &str) -> bool {
    let paren = format!("{}(", target);
    let bytes = line.as_bytes();
    let mut pos = 0;
    while let Some(idx) = line[pos..].find(&paren) {
        let abs = pos + idx;
        if abs == 0 || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_') {
            return true;
        }
        pos = abs + 1;
    }
    line.contains(&format!("::{}", target))
}

fn callees_in_body(def: &FnDef, files: &[(String, String)]) -> Vec<(String, usize)> {
    let content = match files.iter().find(|(p, _)| *p == def.file) {
        Some((_, c)) => c, None => return Vec::new(),
    };
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let start = def.line.saturating_sub(1);
    for i in start..def.end_line.min(lines.len()) {
        let bytes = lines[i].as_bytes();
        for j in 1..bytes.len() {
            if bytes[j] != b'(' { continue; }
            let mut k = j;
            while k > 0 && (bytes[k - 1].is_ascii_alphanumeric() || bytes[k - 1] == b'_') { k -= 1; }
            if j <= k + 1 { continue; }
            let name = &lines[i][k..j];
            if !is_noise(name) && seen.insert(name.to_string()) {
                result.push((name.to_string(), i + 1));
            }
        }
    }
    result
}

fn is_noise(s: &str) -> bool {
    matches!(s, "if" | "for" | "while" | "match" | "return" | "let" | "Some" | "None"
        | "Ok" | "Err" | "Box" | "Vec" | "String" | "format" | "write" | "writeln"
        | "println" | "eprintln" | "assert" | "assert_eq" | "panic" | "todo"
        | "fn" | "pub" | "use" | "mod" | "impl" | "self" | "as" | "in" | "unsafe"
        | "async" | "move" | "type" | "where" | "mut" | "ref" | "true" | "false")
}
