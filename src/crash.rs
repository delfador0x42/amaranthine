//! Crash trace analysis: parse stack frames from crash/error text, find definitions
//! in codebase, annotate causal chain with code context and crash cause patterns.

use std::fmt::Write;
use std::path::Path;

struct Frame {
    func: String,
    file: Option<String>,
    line: Option<usize>,
}

pub fn run(input: &str, path: &Path, glob_suffix: &str) -> Result<String, String> {
    if input.is_empty() { return Err("input (crash/stack trace text) is required".into()); }
    if !path.is_dir() { return Err(format!("{} is not a directory", path.display())); }

    let suffix = glob_suffix.trim_start_matches('*');
    let mut fps = Vec::new();
    crate::codepath::walk_files(path, suffix, &mut fps)?;
    fps.sort();

    // Load all files
    let mut files: Vec<(String, String)> = Vec::new();
    for fp in &fps {
        let content = match std::fs::read_to_string(fp) { Ok(c) => c, Err(_) => continue };
        let rel = fp.strip_prefix(path).unwrap_or(fp).to_string_lossy().to_string();
        files.push((rel, content));
    }

    let frames = parse_frames(input);
    if frames.is_empty() {
        return Err("no stack frames found in input".into());
    }

    let mut out = String::new();
    let error_preview: String = input.lines().next().unwrap_or("unknown").chars().take(60).collect();
    let _ = writeln!(out, "=== CRASH: \"{}\" ===\n", error_preview);

    let _ = writeln!(out, "CHAIN ({} frames):", frames.len());
    for (i, frame) in frames.iter().enumerate() {
        let _ = write!(out, "  [{}] {}", i, frame.func);
        if let Some(ref f) = frame.file {
            let _ = write!(out, " ({}{})", f,
                frame.line.map(|l| format!(":{l}")).unwrap_or_default());
        }
        let _ = writeln!(out);

        // Find definition in codebase
        if let Some((rel, content)) = find_fn_in_files(&frame.func, &files) {
            let lines: Vec<&str> = content.lines().collect();
            // Find the function definition line
            if let Some(def_line) = lines.iter().enumerate()
                .find(|(_, l)| l.contains("fn ") && l.contains(&frame.func))
                .map(|(i, _)| i)
            {
                let start = def_line.saturating_sub(1);
                let end = (def_line + 8).min(lines.len());
                let _ = writeln!(out, "    DEF: {}:{}", rel, def_line + 1);
                for li in start..end {
                    let marker = if li == def_line { ">" } else { " " };
                    let _ = writeln!(out, "    {marker}{:>4} {}", li + 1, lines[li]);
                }

                // Scan function body for crash cause patterns
                let body_end = (def_line + 50).min(lines.len());
                let patterns = scan_crash_patterns(&lines[def_line..body_end]);
                if !patterns.is_empty() {
                    let _ = writeln!(out, "    SUSPECTS:");
                    for (pat_line, pattern) in &patterns {
                        let _ = writeln!(out, "      L{} — {}", def_line + 1 + pat_line, pattern);
                    }
                }
            }
        }
        let _ = writeln!(out);
    }

    // Root cause analysis — focus on crash site function only
    let _ = writeln!(out, "ROOT CAUSE ANALYSIS:");
    let crash_site = frames.first();
    if let Some(site) = crash_site {
        if let Some((rel, content)) = find_fn_in_files(&site.func, &files) {
            let lines: Vec<&str> = content.lines().collect();
            if let Some(def_line) = lines.iter().enumerate()
                .find(|(_, l)| l.contains("fn ") && l.contains(&site.func))
                .map(|(i, _)| i)
            {
                let body_end = (def_line + 50).min(lines.len());
                let patterns = scan_crash_patterns(&lines[def_line..body_end]);
                if patterns.is_empty() {
                    let _ = writeln!(out, "  No obvious crash patterns in {}.", site.func);
                    let _ = writeln!(out, "  SUGGESTION: Check caller context and input validation.");
                } else {
                    for (off, desc) in &patterns {
                        let _ = writeln!(out, "  L{} in {}: {desc}", def_line + 1 + off, rel);
                    }
                }
            }
        } else {
            let _ = writeln!(out, "  Function '{}' not found in codebase.", site.func);
            let _ = writeln!(out, "  NOTE: May be in a dependency or standard library.");
        }
    }

    Ok(out)
}

fn parse_frames(input: &str) -> Vec<Frame> {
    let mut frames = Vec::new();
    for line in input.lines() {
        let t = line.trim();
        // Pattern: "N  module  address  function + offset"
        // Pattern: "at file:line"
        // Pattern: "in function (file:line)"
        // Pattern: "function_name(args)" or "module::function"
        // Pattern: Rust backtrace "N: function_name"
        // Pattern: "thread 'X' panicked at 'msg', file:line"

        if let Some(frame) = parse_rust_backtrace_line(t) {
            frames.push(frame);
        } else if let Some(frame) = parse_generic_frame(t) {
            frames.push(frame);
        }
    }
    frames
}

fn parse_rust_backtrace_line(line: &str) -> Option<Frame> {
    // "   N: module::function" or "N: function_name"
    let stripped = line.trim_start_matches(|c: char| c.is_ascii_digit() || c == ':' || c == ' ');
    if stripped.len() == line.len() { return None; } // no leading number
    let func_part = stripped.trim();
    if func_part.is_empty() { return None; }

    // Extract "at file:line" if present
    let (func, file, line_no) = if let Some(idx) = func_part.find(" at ") {
        let func = &func_part[..idx];
        let loc = &func_part[idx + 4..];
        let (f, l) = parse_file_line(loc);
        (func, f, l)
    } else {
        (func_part, None, None)
    };

    // Take last segment of module path for function name
    let name = func.rsplit("::").next().unwrap_or(func);
    if name.len() < 2 || is_stdlib(name) { return None; }

    Some(Frame { func: name.to_string(), file, line: line_no })
}

fn parse_generic_frame(line: &str) -> Option<Frame> {
    // "in function_name" or "function_name (file:line)"
    let stripped = if let Some(rest) = line.strip_prefix("in ") { rest }
        else if line.contains("panicked at") {
            // "thread 'X' panicked at 'msg', file:line"
            let parts: Vec<&str> = line.rsplitn(2, ", ").collect();
            if parts.len() == 2 {
                return parse_file_line(parts[0]).0.map(|f| Frame {
                    func: "panic".into(), file: Some(f), line: parse_file_line(parts[0]).1,
                });
            }
            return None;
        }
        else { return None; };

    let (func, file, line_no) = if let Some(paren) = stripped.find('(') {
        let func = &stripped[..paren];
        let rest = &stripped[paren + 1..stripped.len().saturating_sub(1)];
        let (f, l) = parse_file_line(rest);
        (func.trim(), f, l)
    } else {
        (stripped.trim(), None, None)
    };

    if func.is_empty() || func.len() < 2 { return None; }
    Some(Frame { func: func.to_string(), file, line: line_no })
}

fn parse_file_line(s: &str) -> (Option<String>, Option<usize>) {
    if let Some(colon) = s.rfind(':') {
        let file = &s[..colon];
        let line = s[colon + 1..].parse::<usize>().ok();
        if !file.is_empty() { (Some(file.to_string()), line) }
        else { (None, None) }
    } else if !s.is_empty() {
        (Some(s.to_string()), None)
    } else {
        (None, None)
    }
}

fn find_fn_in_files<'a>(name: &str, files: &'a [(String, String)]) -> Option<(&'a str, &'a str)> {
    for (rel, content) in files {
        if content.contains(&format!("fn {name}")) {
            return Some((rel.as_str(), content.as_str()));
        }
    }
    None
}

fn scan_crash_patterns(lines: &[&str]) -> Vec<(usize, String)> {
    let mut patterns = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("//") { continue; }
        if t.contains(".unwrap()") {
            patterns.push((i, "unwrap() — panics on None/Err".into()));
        } else if t.contains(".expect(") {
            patterns.push((i, "expect() — panics with message on None/Err".into()));
        } else if t.contains("[") && t.contains("]") && !t.contains("//") {
            // Check for array/slice indexing (bounds risk)
            if t.contains("as usize]") || t.contains("idx]") || t.contains("index]")
                || t.contains("i]") || t.contains("pos]") {
                patterns.push((i, "array index — potential bounds panic".into()));
            }
        }
        if t.contains("as u") && (t.contains("as u8") || t.contains("as u16") || t.contains("as u32")) {
            patterns.push((i, "integer truncation cast".into()));
        }
        if t.contains("unsafe") {
            patterns.push((i, "unsafe block — memory safety boundary".into()));
        }
    }
    patterns
}

fn is_stdlib(name: &str) -> bool {
    matches!(name, "main" | "call_once" | "call" | "invoke" | "start_thread"
        | "clone" | "drop" | "deref" | "fmt" | "write" | "read" | "poll"
        | "resume" | "catch_unwind" | "panic" | "begin_panic")
}
