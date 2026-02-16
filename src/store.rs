use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub fn run(dir: &Path, topic: &str, text: &str) -> Result<(), String> {
    crate::config::ensure_dir(dir)?;

    let filename = sanitize(topic);
    let filepath = dir.join(format!("{filename}.md"));
    let timestamp = Local::now().format("%Y-%m-%d %H:%M");
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
    println!("stored in {filename}.md ({count} entries)");
    Ok(())
}

fn sanitize(topic: &str) -> String {
    topic
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect()
}

fn count_entries(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| l.starts_with("## ")).count())
        .unwrap_or(0)
}
