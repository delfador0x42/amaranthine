use std::fmt::Write;
use std::fs;
use std::path::Path;

/// Scan for entries without proper timestamps and optionally fix them.
pub fn run(dir: &Path, apply: bool) -> Result<String, String> {
    let files = crate::config::list_topic_files(dir)?;
    let mut out = String::new();
    let mut total_fixed = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let mut needs_fix = false;

        for line in content.lines() {
            if line.starts_with("## ") {
                let rest = line.trim_start_matches("## ");
                if crate::time::parse_date_days(rest).is_none() {
                    needs_fix = true;
                    let _ = writeln!(out, "  [{name}] bad header: {line}");
                }
            }
        }

        if needs_fix && apply {
            let now = crate::time::LocalTime::now();
            let fixed = fix_timestamps(&content, &format!("{now}"));
            crate::config::atomic_write(path, &fixed)?;
            total_fixed += 1;
            let _ = writeln!(out, "  [{name}] fixed timestamps");
        }
    }

    if out.is_empty() {
        let _ = writeln!(out, "all entries have valid timestamps");
    } else if !apply {
        let _ = writeln!(out, "\nrun with apply=true to backfill timestamps");
    } else {
        let _ = writeln!(out, "\nfixed {total_fixed} topic(s)");
    }
    Ok(out)
}

fn fix_timestamps(content: &str, fallback: &str) -> String {
    let mut result = String::new();
    for line in content.lines() {
        if line.starts_with("## ") {
            let rest = line.trim_start_matches("## ");
            if crate::time::parse_date_days(rest).is_none() {
                // Preserve original text as first line of body, add timestamp
                result.push_str(&format!("## {fallback}\n"));
                result.push_str(&format!("{rest}\n"));
                continue;
            }
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}
