use crate::time;
use std::fmt::Write;
use std::path::Path;

pub fn run(dir: &Path, stale_days: u64, plain: bool) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Ok("no data.log found\n".into()); }
    crate::cache::with_corpus(dir, |cached| {
        let today = time::LocalTime::now().to_days();
        let cutoff = today - stale_days as i64;

        // Find newest entry per topic
        let mut newest: std::collections::BTreeMap<&str, i64> = std::collections::BTreeMap::new();
        for e in cached {
            let days = e.timestamp_min as i64 / 1440;
            let cur = newest.entry(&e.topic).or_insert(0);
            if days > *cur { *cur = days; }
        }

        let mut stale = 0;
        let mut out = String::new();
        for (name, latest) in &newest {
            if *latest == 0 {
                if plain { let _ = writeln!(out, "no dates: {name}"); }
                else { let _ = writeln!(out, "\x1b[1;31mno dates:\x1b[0m {name}"); }
                stale += 1;
            } else if *latest < cutoff {
                if plain { let _ = writeln!(out, "stale: {name} (last entry > {stale_days} days ago)"); }
                else { let _ = writeln!(out, "\x1b[1;33mstale:\x1b[0m {name} (> {stale_days} days)"); }
                stale += 1;
            }
        }
        if stale == 0 {
            let _ = writeln!(out, "nothing stale (threshold: {stale_days} days)");
        } else {
            let _ = writeln!(out, "\n{stale} stale topic(s) — review manually");
        }
        out
    })
}
