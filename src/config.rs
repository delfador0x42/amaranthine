use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, fs};

pub fn resolve_dir(explicit: Option<String>) -> PathBuf {
    if let Some(d) = explicit {
        return PathBuf::from(d);
    }
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".amaranthine")
}

pub fn init(path: Option<&str>) -> Result<(), String> {
    let dir = match path {
        Some(p) => PathBuf::from(p),
        None => resolve_dir(None),
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

pub fn sanitize_topic(topic: &str) -> String {
    topic
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect()
}

/// Topic files only (excludes INDEX.md and MEMORY.md).
pub fn list_topic_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    list_md_files(dir, &["INDEX.md", "MEMORY.md"])
}

/// All searchable .md files (excludes INDEX.md only).
pub fn list_search_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    list_md_files(dir, &["INDEX.md"])
}

/// Atomic file write: write to .tmp, fsync, rename over target.
/// Prevents corruption if process dies mid-write.
pub fn atomic_write(path: &Path, data: &str) -> Result<(), String> {
    let tmp = path.with_extension("md.tmp");
    let mut f = fs::File::create(&tmp)
        .map_err(|e| format!("can't create {}: {e}", tmp.display()))?;
    f.write_all(data.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;
    f.sync_all()
        .map_err(|e| format!("fsync failed: {e}"))?;
    drop(f);
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename failed: {e}"))?;
    Ok(())
}

/// Parse [source: path/to/file:line] from entry lines.
pub fn parse_source(lines: &[&str]) -> Option<(String, Option<usize>)> {
    for line in lines {
        if let Some(inner) = line.strip_prefix("[source: ").and_then(|s| s.strip_suffix(']')) {
            let inner = inner.trim();
            if let Some((path, line_num)) = inner.rsplit_once(':') {
                if let Ok(n) = line_num.parse::<usize>() {
                    return Some((path.to_string(), Some(n)));
                }
            }
            return Some((inner.to_string(), None));
        }
    }
    None
}

/// Check if a source file is newer than the entry timestamp.
pub fn check_staleness(source: &str, entry_header: &str) -> Option<String> {
    let entry_secs = crate::time::parse_date_minutes(entry_header)? * 60;
    let mtime = fs::metadata(source).ok()?.modified().ok()?;
    let file_secs = mtime.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    if file_secs > entry_secs {
        Some("STALE (source modified after entry)".into())
    } else {
        None
    }
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
