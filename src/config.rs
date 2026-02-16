use std::path::{Path, PathBuf};
use std::{env, fs};

/// Resolve memory directory: explicit > walk-up .amaranthine > ~/.amaranthine
pub fn resolve_dir(explicit: Option<String>) -> PathBuf {
    if let Some(d) = explicit {
        return PathBuf::from(d);
    }
    if let Ok(cwd) = env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            let candidate = dir.join(".amaranthine");
            if candidate.is_dir() {
                return candidate;
            }
            match dir.parent() {
                Some(p) => dir = p,
                None => break,
            }
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".amaranthine")
}

pub fn init(path: Option<String>) -> Result<(), String> {
    let dir = match path {
        Some(p) => PathBuf::from(p),
        None => env::current_dir()
            .map_err(|e| e.to_string())?
            .join(".amaranthine"),
    };
    fs::create_dir_all(&dir)
        .map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    println!("initialized: {}", dir.display());
    Ok(())
}

pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("{} doesn't exist, can't create: {e}", dir.display()))?;
    }
    Ok(())
}

/// Topic files only (excludes INDEX.md and MEMORY.md)
pub fn list_topic_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    list_md_files(dir, &["INDEX.md", "MEMORY.md"])
}

/// All searchable files (excludes INDEX.md only)
pub fn list_search_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    list_md_files(dir, &["INDEX.md"])
}

fn list_md_files(dir: &Path, exclude: &[&str]) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .filter(|p| {
            p.file_name()
                .map(|n| !exclude.iter().any(|ex| *ex == n.to_string_lossy()))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    Ok(files)
}
