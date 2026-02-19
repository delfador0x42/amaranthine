use crate::json::Value;
use std::path::Path;

/// Export all topics as structured JSON from cached corpus.
pub fn export(dir: &Path) -> Result<String, String> {
    crate::cache::with_corpus(dir, |cached| {
        // Group by topic, preserving insertion order
        let mut topic_order: Vec<String> = Vec::new();
        let mut grouped: std::collections::BTreeMap<&str, Vec<&crate::cache::CachedEntry>> =
            std::collections::BTreeMap::new();
        for e in cached {
            if !grouped.contains_key(e.topic.as_str()) { topic_order.push(e.topic.to_string()); }
            grouped.entry(e.topic.as_str()).or_default().push(e);
        }

        let mut topics: Vec<Value> = Vec::new();
        for name in &topic_order {
            let group = &grouped[name.as_str()];
            let entries: Vec<Value> = group.iter().map(|e| {
                let date = crate::time::minutes_to_date_str(e.timestamp_min);
                let mut tags_list: Vec<Value> = Vec::new();
                let mut body_lines: Vec<&str> = Vec::new();
                for line in e.body.lines() {
                    if let Some(inner) = line.strip_prefix("[tags: ").and_then(|s| s.strip_suffix(']')) {
                        for tag in inner.split(',') {
                            let t = tag.trim();
                            if !t.is_empty() { tags_list.push(Value::Str(t.into())); }
                        }
                    } else { body_lines.push(line); }
                }
                Value::Obj(vec![
                    ("timestamp".into(), Value::Str(date)),
                    ("tags".into(), Value::Arr(tags_list)),
                    ("body".into(), Value::Str(body_lines.join("\n").trim().to_string())),
                ])
            }).collect();
            topics.push(Value::Obj(vec![
                ("topic".into(), Value::Str(name.clone())),
                ("entries".into(), Value::Arr(entries)),
            ]));
        }

        let root = Value::Obj(vec![
            ("version".into(), Value::Str("4.0.0".into())),
            ("topics".into(), Value::Arr(topics)),
        ]);
        root.pretty()
    })
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
                    let t: Vec<&str> = items.iter().filter_map(|i| i.as_str()).collect();
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
