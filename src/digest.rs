use std::fs;
use std::path::Path;

pub fn run(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let files = crate::config::list_topic_files(dir)?;
    if files.is_empty() {
        println!("no topics");
        return Ok(());
    }

    for (i, path) in files.iter().enumerate() {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy();

        let title = content
            .lines()
            .find(|l| l.starts_with("# ") && !l.starts_with("## "))
            .map(|l| l.trim_start_matches("# ").trim())
            .unwrap_or(&name);

        let headers: Vec<&str> = content
            .lines()
            .filter(|l| l.starts_with("## "))
            .collect();
        let count = headers.len();
        let latest = headers
            .last()
            .map(|h| h.trim_start_matches("## "))
            .unwrap_or("empty");

        if i > 0 { println!(); }
        println!("### {title} ({count} entries, last: {latest})");

        // First non-empty content line per section = summary bullet
        let mut in_section = false;
        let mut got_summary = false;
        for line in content.lines() {
            if line.starts_with("## ") {
                in_section = true;
                got_summary = false;
            } else if in_section && !got_summary && !line.is_empty() {
                let trimmed = line.trim_start_matches("- ").trim();
                if !trimmed.is_empty() {
                    println!("- {}", truncate(trimmed, 100));
                    got_summary = true;
                }
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}
