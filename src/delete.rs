use std::fs;
use std::path::Path;

pub fn run(dir: &Path, topic: &str, last: bool, all: bool, match_str: Option<&str>) -> Result<(), String> {
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

    if let Some(needle) = match_str {
        return delete_matching(&filepath, &filename, needle);
    }

    if !last {
        return Err("specify --last, --all, or --match <substring>".into());
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

fn delete_matching(filepath: &Path, filename: &str, needle: &str) -> Result<(), String> {
    let content = fs::read_to_string(filepath).map_err(|e| e.to_string())?;
    let sections = split_sections(&content);
    let lower = needle.to_lowercase();

    let idx = sections.iter().position(|(_, body)| body.to_lowercase().contains(&lower));
    let idx = match idx {
        Some(i) => i,
        None => return Err(format!("no entry matching \"{needle}\"")),
    };

    let mut result = sections.iter().enumerate()
        .filter(|(i, _)| *i != idx)
        .map(|(_, (hdr, body))| format!("{hdr}\n{body}"))
        .collect::<Vec<_>>()
        .join("");

    // Preserve the # title header if present
    if let Some(title_end) = content.find("\n## ") {
        let header = &content[..title_end + 1];
        if !result.starts_with("# ") {
            result = format!("{header}{result}");
        }
    }

    if result.trim().lines().count() <= 1 {
        // Only title left — just keep the title
        let title = content.lines().next().unwrap_or("");
        fs::write(filepath, format!("{title}\n\n")).map_err(|e| e.to_string())?;
    } else {
        if !result.ends_with('\n') { result.push('\n'); }
        fs::write(filepath, &result).map_err(|e| e.to_string())?;
    }

    let remaining = result.matches("\n## ").count();
    println!("removed entry matching \"{needle}\" from {filename}.md ({remaining} remaining)");
    Ok(())
}

/// Split content into (header_line, body) pairs for each `## ` section.
/// Skips the `# title` header at the top.
pub fn split_sections(content: &str) -> Vec<(&str, &str)> {
    let mut sections = Vec::new();
    let mut i = 0;
    let bytes = content.as_bytes();

    while i < bytes.len() {
        // Find next ## header
        let hdr_start = if content[i..].starts_with("## ") {
            Some(i)
        } else {
            content[i..].find("\n## ").map(|p| i + p + 1)
        };

        let hdr_start = match hdr_start {
            Some(s) => s,
            None => break,
        };

        let hdr_end = content[hdr_start..].find('\n')
            .map(|p| hdr_start + p)
            .unwrap_or(content.len());

        let header = &content[hdr_start..hdr_end];

        // Body extends to next ## or end
        let body_end = content[hdr_end..].find("\n## ")
            .map(|p| hdr_end + p + 1)
            .unwrap_or(content.len());

        let body = &content[hdr_end..body_end];
        sections.push((header, body));
        i = body_end;
    }
    sections
}
