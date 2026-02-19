use std::fmt::Write;
use std::path::Path;

/// Scan data.log for entries without timestamps (timestamp_min == 0).
/// Optionally fix by re-appending with current timestamp + tombstoning old.
pub fn run(dir: &Path, apply: bool) -> Result<String, String> {
    let log_path = crate::config::log_path(dir);
    if !log_path.exists() { return Ok("no data.log found\n".into()); }
    let entries = crate::datalog::iter_live(&log_path)?;

    let mut out = String::new();
    let mut total = 0;

    for e in &entries {
        if e.timestamp_min != 0 { continue; }
        let preview = e.body.lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with("[tags:"))
            .map(|l| { let t = l.trim(); if t.len() > 60 { &t[..60] } else { t } })
            .unwrap_or("(empty)");
        let _ = writeln!(out, "  [{}] no timestamp: {preview}", e.topic);
        total += 1;

        if apply {
            let ts = crate::time::LocalTime::now().to_minutes() as i32;
            crate::datalog::append_entry(&log_path, &e.topic, &e.body, ts)?;
            crate::datalog::append_delete(&log_path, e.offset)?;
        }
    }

    if total == 0 {
        let _ = writeln!(out, "all entries have valid timestamps");
    } else if !apply {
        let _ = writeln!(out, "\n{total} entries without timestamps");
        let _ = writeln!(out, "run with apply=true to backfill with current time");
    } else {
        let _ = writeln!(out, "\nfixed {total} entries with current timestamp");
    }
    Ok(out)
}
