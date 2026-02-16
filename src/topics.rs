use crate::time;
use std::fs;
use std::path::Path;

pub fn list(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let files = crate::config::list_topic_files(dir)?;
    if files.is_empty() {
        println!("no topic files in {}", dir.display());
        return Ok(());
    }

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();
        let entries = content.lines().filter(|l| l.starts_with("## ")).count();
        let lines = content.lines().count();
        println!("  {name:<24} {entries:>3} entries  {lines:>4} lines");
    }
    Ok(())
}

pub fn recent(dir: &Path, days: u64, plain: bool) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let today = time::LocalTime::now().to_days();
    let cutoff = today - days as i64;
    let files = crate::config::list_topic_files(dir)?;
    let mut found = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();
        let mut in_recent = false;

        for line in content.lines() {
            if line.starts_with("## ") {
                let header = line.trim_start_matches("## ");
                in_recent = time::parse_date_days(header)
                    .map(|d| d >= cutoff)
                    .unwrap_or(false);
                if in_recent {
                    if plain {
                        println!("[{name}] {line}");
                    } else {
                        println!("\x1b[1;36m[{name}]\x1b[0m {line}");
                    }
                    found += 1;
                }
            } else if in_recent && !line.is_empty() {
                println!("  {line}");
            }
        }
    }

    if found == 0 {
        println!("no entries in the last {days} days");
    }
    Ok(())
}
