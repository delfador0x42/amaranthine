use crate::time::LocalTime;
use std::fs;
use std::path::Path;

pub fn run(dir: &Path) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let files = crate::config::list_topic_files(dir)?;
    let mut topics: Vec<(String, usize, usize, String)> = Vec::new();

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let entries = content.lines().filter(|l| l.starts_with("## ")).count();
        let lines = content.lines().count();
        let last = content
            .lines()
            .filter(|l| l.starts_with("## "))
            .last()
            .map(|l| l.trim_start_matches("## ").to_string())
            .unwrap_or_default();
        topics.push((name, entries, lines, last));
    }

    let total: usize = topics.iter().map(|t| t.1).sum();
    let now = LocalTime::now();

    let mut out = format!("# Amaranthine Index\nGenerated: {now}\n\n");
    out += &format!("## Topics ({} files, {total} entries)\n", topics.len());

    for (name, entries, lines, last) in &topics {
        out += &format!("- **{name}** — {entries} entries, {lines} lines (last: {last})\n");
    }

    let index_path = dir.join("INDEX.md");
    fs::write(&index_path, &out).map_err(|e| e.to_string())?;
    out += &format!("\nwritten to {}", index_path.display());
    Ok(out)
}
