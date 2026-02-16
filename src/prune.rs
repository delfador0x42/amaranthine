use chrono::{Days, Local, NaiveDate};
use std::fs;
use std::path::Path;

pub fn run(dir: &Path, stale_days: u64) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let cutoff = Local::now()
        .date_naive()
        .checked_sub_days(Days::new(stale_days))
        .unwrap();

    let files = crate::config::list_topic_files(dir)?;
    let mut stale = 0;

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();

        let latest = content
            .lines()
            .filter(|l| l.starts_with("## "))
            .filter_map(|l| {
                let s = l.trim_start_matches("## ").split_whitespace().next()?;
                NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
            })
            .max();

        match latest {
            Some(date) if date < cutoff => {
                println!("\x1b[1;33mstale:\x1b[0m {name} (last: {date})");
                stale += 1;
            }
            None => {
                println!("\x1b[1;31mno dates:\x1b[0m {name}");
                stale += 1;
            }
            _ => {}
        }
    }

    if stale == 0 {
        println!("nothing stale (threshold: {stale_days} days)");
    } else {
        println!("\n{stale} stale topic(s) — review manually");
    }
    Ok(())
}
