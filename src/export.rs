use crate::json::Value;
use std::fs;
use std::path::Path;

/// Export all topics as structured JSON.
pub fn export(dir: &Path) -> Result<String, String> {
    let files = crate::config::list_topic_files(dir)?;
    let mut topics: Vec<Value> = Vec::new();

    for path in &files {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let sections = crate::delete::split_sections(&content);

        let entries: Vec<Value> = sections.iter().map(|(hdr, body)| {
            let timestamp = hdr.strip_prefix("## ").unwrap_or("").to_string();
            let mut tags_list: Vec<Value> = Vec::new();
            let mut body_lines: Vec<&str> = Vec::new();

            for line in body.lines() {
                if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                    for tag in inner.split(',') {
                        let t = tag.trim();
                        if !t.is_empty() { tags_list.push(Value::Str(t.into())); }
                    }
                } else {
                    body_lines.push(line);
                }
            }

            let body_text = body_lines.join("\n").trim().to_string();
            Value::Obj(vec![
                ("timestamp".into(), Value::Str(timestamp)),
                ("tags".into(), Value::Arr(tags_list)),
                ("body".into(), Value::Str(body_text)),
            ])
        }).collect();

        topics.push(Value::Obj(vec![
            ("topic".into(), Value::Str(name)),
            ("entries".into(), Value::Arr(entries)),
        ]));
    }

    let root = Value::Obj(vec![
        ("version".into(), Value::Str("0.9.0".into())),
        ("topics".into(), Value::Arr(topics)),
    ]);
    Ok(root.pretty())
}

/// Import topics from JSON (merges with existing — does not overwrite).
pub fn import(dir: &Path, json_str: &str) -> Result<String, String> {
    crate::config::ensure_dir(dir)?;
    let root = crate::json::parse(json_str).map_err(|e| format!("bad JSON: {e}"))?;
    let topics = root.get("topics").ok_or("missing 'topics' array")?;

    let arr = match topics {
        Value::Arr(items) => items,
        _ => return Err("'topics' must be an array".into()),
    };

    let mut imported = 0;
    for item in arr {
        let topic = item.get("topic").and_then(|v| v.as_str()).unwrap_or("unknown");
        let entries = match item.get("entries") {
            Some(Value::Arr(e)) => e,
            _ => continue,
        };
        for entry in entries {
            let body = entry.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let tags_val = entry.get("tags");
            let tags: Option<String> = tags_val.and_then(|v| match v {
                Value::Arr(items) => {
                    let t: Vec<&str> = items.iter()
                        .filter_map(|i| i.as_str())
                        .collect();
                    if t.is_empty() { None } else { Some(t.join(",")) }
                }
                _ => None,
            });
            crate::store::run_with_tags(dir, topic, body, tags.as_deref())?;
            imported += 1;
        }
    }
    Ok(format!("imported {imported} entries across {} topics", arr.len()))
}
