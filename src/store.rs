use crate::time::LocalTime;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::Path;

pub fn run(dir: &Path, topic: &str, text: &str) -> Result<String, String> {
    crate::config::ensure_dir(dir)?;

    let text = if text == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).map_err(|e| e.to_string())?;
        let trimmed = buf.trim_end();
        if trimmed.is_empty() {
            return Err("empty stdin".into());
        }
        trimmed.to_string()
    } else {
        text.to_string()
    };

    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));
    let timestamp = LocalTime::now();
    let is_new = !filepath.exists();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&filepath)
        .map_err(|e| format!("can't open {}: {e}", filepath.display()))?;

    if is_new {
        writeln!(file, "# {topic}\n").map_err(|e| e.to_string())?;
    }
    writeln!(file, "## {timestamp}").map_err(|e| e.to_string())?;
    writeln!(file, "{text}\n").map_err(|e| e.to_string())?;

    let count = count_entries(&filepath);
    Ok(format!("stored in {filename}.md ({count} entries)"))
}

fn count_entries(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| l.starts_with("## ")).count())
        .unwrap_or(0)
}
