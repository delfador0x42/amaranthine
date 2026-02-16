use crate::time;
use std::fs;
use std::path::Path;

pub fn run(dir: &Path, stale_days: u64, plain: bool) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let today = time::LocalTime::now().to_days();
    let cutoff = today - stale_days as i64;
    let files = crate::config::list_topic_files(dir)?;
    let mut stale = 0;
    let mut out = String::new();

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();

        let latest = content
            .lines()
            .filter(|l| l.starts_with("## "))
            .filter_map(|l| time::parse_date_days(l.trim_start_matches("## ")))
            .max();

        match latest {
            Some(d) if d < cutoff => {
                if plain {
                    out += &format!("stale: {name} (last entry > {stale_days} days ago)\n");
                } else {
                    out += &format!("\x1b[1;33mstale:\x1b[0m {name} (> {stale_days} days)\n");
                }
                stale += 1;
            }
            None => {
                if plain {
                    out += &format!("no dates: {name}\n");
                } else {
                    out += &format!("\x1b[1;31mno dates:\x1b[0m {name}\n");
                }
                stale += 1;
            }
            _ => {}
        }
    }

    if stale == 0 {
        out += &format!("nothing stale (threshold: {stale_days} days)\n");
    } else {
        out += &format!("\n{stale} stale topic(s) — review manually\n");
    }
    Ok(out)
}
