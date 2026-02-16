use std::fs;
use std::path::Path;

pub fn run(dir: &Path, topic: &str, last: bool, all: bool) -> Result<(), String> {
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    if all {
        fs::remove_file(&filepath).map_err(|e| e.to_string())?;
        println!("deleted {filename}.md");
        return Ok(());
    }

    if !last {
        return Err("specify --last or --all".into());
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    match content.rfind("\n## ") {
        Some(pos) => {
            let trimmed = content[..pos].trim_end();
            fs::write(&filepath, format!("{trimmed}\n")).map_err(|e| e.to_string())?;
            let remaining = trimmed.matches("\n## ").count();
            println!("removed last entry from {filename}.md ({remaining} remaining)");
        }
        None => return Err("no entries to remove".into()),
    }
    Ok(())
}
