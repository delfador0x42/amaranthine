use std::fmt::Write;
use std::fs;
use std::path::Path;

pub fn run(dir: &Path, query: &str, plain: bool) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let query_lower = query.to_lowercase();
    let files = crate::config::list_search_files(dir)?;
    let mut total = 0;
    let mut out = String::new();

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_name().unwrap().to_string_lossy();
        let sections = parse_sections(&content);
        let mut file_printed = false;

        for section in &sections {
            if section.iter().any(|l| l.to_lowercase().contains(&query_lower)) {
                if !file_printed {
                    if plain {
                        let _ = writeln!(out, "\n--- {name} ---");
                    } else {
                        let _ = writeln!(out, "\n\x1b[1;36m--- {name} ---\x1b[0m");
                    }
                    file_printed = true;
                }
                for line in section {
                    if line.to_lowercase().contains(&query_lower) {
                        if plain {
                            let _ = writeln!(out, "> {line}");
                        } else {
                            let _ = writeln!(out, "\x1b[1;33m{line}\x1b[0m");
                        }
                    } else {
                        let _ = writeln!(out, "{line}");
                    }
                }
                let _ = writeln!(out);
                total += 1;
            }
        }
    }

    if total == 0 {
        let _ = writeln!(out, "no matches for '{query}'");
    } else {
        let _ = writeln!(out, "{total} matching section(s)");
    }
    Ok(out)
}

fn parse_sections(content: &str) -> Vec<Vec<&str>> {
    let mut sections: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in content.lines() {
        if line.starts_with("## ") && !current.is_empty() {
            sections.push(current);
            current = Vec::new();
        }
        current.push(line);
    }
    if !current.is_empty() {
        sections.push(current);
    }
    sections
}
