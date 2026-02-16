use std::fs;
use std::path::Path;

/// Replace the content of the first entry matching `needle` with `new_text`.
/// Keeps the original timestamp header.
pub fn run(dir: &Path, topic: &str, needle: &str, new_text: &str) -> Result<String, String> {
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

    let result = crate::delete::rebuild_file(&content, &sections, None, Some((idx, new_text)));
    fs::write(&filepath, &result).map_err(|e| e.to_string())?;
    Ok(format!("updated entry matching \"{needle}\" in {filename}.md"))
}
