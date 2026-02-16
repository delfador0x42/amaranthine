use std::fs;
use std::path::Path;

/// Replace the content of the first entry matching `needle` with `new_text`.
/// Keeps the original timestamp header.
pub fn run(dir: &Path, topic: &str, needle: &str, new_text: &str) -> Result<(), String> {
    let filename = crate::config::sanitize_topic(topic);
    let filepath = dir.join(format!("{filename}.md"));

    if !filepath.exists() {
        return Err(format!("{filename}.md not found"));
    }

    let content = fs::read_to_string(&filepath).map_err(|e| e.to_string())?;
    let sections = crate::delete::split_sections(&content);
    let lower = needle.to_lowercase();

    let idx = sections.iter().position(|(_, body)| body.to_lowercase().contains(&lower));
    let idx = match idx {
        Some(i) => i,
        None => return Err(format!("no entry matching \"{needle}\"")),
    };

    // Rebuild: title header + sections with replaced entry
    let title_end = content.find("\n## ").unwrap_or(0);
    let header = if title_end > 0 { &content[..title_end + 1] } else { "" };

    let mut result = String::from(header);
    for (i, (hdr, _)) in sections.iter().enumerate() {
        result.push_str(hdr);
        result.push('\n');
        if i == idx {
            result.push_str(new_text);
            result.push_str("\n\n");
        } else {
            // Preserve original body
            let orig_body = sections[i].1;
            result.push_str(orig_body);
        }
    }

    if !result.ends_with('\n') { result.push('\n'); }
    fs::write(&filepath, &result).map_err(|e| e.to_string())?;
    println!("updated entry matching \"{needle}\" in {filename}.md");
    Ok(())
}
