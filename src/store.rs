use crate::time::LocalTime;
use std::io::{self, Read};
use std::path::Path;

pub fn run(dir: &Path, topic: &str, text: &str) -> Result<String, String> {
    run_full(dir, topic, text, None, false, None)
}

pub fn run_with_tags(dir: &Path, topic: &str, text: &str, tags: Option<&str>) -> Result<String, String> {
    run_full(dir, topic, text, tags, false, None)
}

pub fn run_full(
    dir: &Path, topic: &str, text: &str, tags: Option<&str>,
    force: bool, source: Option<&str>,
) -> Result<String, String> {
    run_full_ext(dir, topic, text, tags, force, source, None, None)
}

pub fn run_full_conf(
    dir: &Path, topic: &str, text: &str, tags: Option<&str>,
    force: bool, source: Option<&str>, confidence: Option<f64>,
) -> Result<String, String> {
    run_full_ext(dir, topic, text, tags, force, source, confidence, None)
}

pub fn run_full_ext(
    dir: &Path, topic: &str, text: &str, tags: Option<&str>,
    force: bool, source: Option<&str>, confidence: Option<f64>,
    links: Option<&str>,
) -> Result<String, String> {
    crate::config::ensure_dir(dir)?;
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let text = read_text(text)?;
    let log_path = crate::datalog::ensure_log(dir)?;

    // Build body with metadata lines. Auto-detect tags from content when none given.
    let cleaned_tags = tags.map(|t| normalize_tags(t))
        .or_else(|| auto_detect_tags(&text));
    let body = build_body(&text, cleaned_tags.as_deref(), source, confidence, links);

    let ts = LocalTime::now();
    let ts_min = ts.to_minutes() as i32;

    // Dupe check
    let dupe_warn = if !force { check_dupe(dir, topic, &text) } else { None };
    let topic_hint = suggest_topic(dir, topic);

    let offset = crate::datalog::append_entry(&log_path, topic, &body, ts_min)?;
    crate::cache::append_to_cache(dir, topic, &body, ts_min, offset);

    let echo = text.lines().map(|l| format!("  > {l}")).collect::<Vec<_>>().join("\n");
    let tag_echo = cleaned_tags.as_deref().filter(|t| !t.is_empty())
        .map(|t| format!(" [tags: {t}]")).unwrap_or_default();
    let conf_echo = confidence.filter(|c| *c < 1.0)
        .map(|c| format!(" (~{:.0}%)", c * 100.0)).unwrap_or_default();
    let link_echo = links.filter(|l| !l.is_empty())
        .map(|l| format!(" [links: {l}]")).unwrap_or_default();
    let mut msg = format!("stored in {topic}\n  @ {ts}{tag_echo}{conf_echo}{link_echo}\n{echo}");
    if let Some(hint) = topic_hint { msg.push_str(&format!("\n  note: {hint}")); }
    if let Some(ref dw) = dupe_warn { msg.push_str(&format!("\n  dupe warning: {dw}")); }
    if let Some(link_str) = links {
        let warn = validate_links(dir, link_str);
        if !warn.is_empty() { msg.push_str(&format!("\n  link warnings: {warn}")); }
    }
    Ok(msg)
}

/// Lean write for batch_store — no lock, no dupe check.
pub fn run_batch_entry(
    dir: &Path, topic: &str, text: &str, tags: Option<&str>, source: Option<&str>,
) -> Result<String, String> {
    crate::config::ensure_dir(dir)?;
    let log_path = crate::datalog::ensure_log(dir)?;
    let cleaned_tags = tags.map(|t| normalize_tags(t));
    let body = build_body(text, cleaned_tags.as_deref(), source, None, None);
    let ts_min = LocalTime::now().to_minutes() as i32;
    crate::datalog::append_entry(&log_path, topic, &body, ts_min)?;
    Ok(format!("stored in {topic}"))
}

/// F3: Lean write using pre-opened file handle — no lock, no dupe check, no fsync.
pub fn run_batch_entry_to(
    f: &mut std::fs::File, topic: &str, text: &str, tags: Option<&str>, source: Option<&str>,
) -> Result<String, String> {
    let cleaned_tags = tags.map(|t| normalize_tags(t));
    let body = build_body(text, cleaned_tags.as_deref(), source, None, None);
    let ts_min = LocalTime::now().to_minutes() as i32;
    crate::datalog::append_entry_to(f, topic, &body, ts_min)?;
    Ok(format!("stored in {topic}"))
}

/// Import entry with explicit timestamp (preserves original dates on import).
pub fn import_entry(
    dir: &Path, topic: &str, body: &str, tags: Option<&str>, ts_min: i32,
) -> Result<String, String> {
    crate::config::ensure_dir(dir)?;
    let log_path = crate::datalog::ensure_log(dir)?;
    let cleaned_tags = tags.map(|t| normalize_tags(t));
    let body = build_body(body, cleaned_tags.as_deref(), None, None, None);
    crate::datalog::append_entry(&log_path, topic, &body, ts_min)?;
    Ok(format!("imported to {topic}"))
}

/// Append text to the last entry in a topic (no new timestamp).
pub fn append(dir: &Path, topic: &str, text: &str) -> Result<String, String> {
    let _lock = crate::lock::FileLock::acquire(dir)?;
    let text = read_text(text)?;
    let log_path = crate::config::log_path(dir);
    let entries = crate::datalog::iter_live(&log_path)?;
    let last = entries.iter().rev().find(|e| e.topic == topic)
        .ok_or_else(|| format!("{topic} not found — use 'store' first"))?;
    let new_body = format!("{}\n{text}", last.body.trim_end());
    crate::datalog::append_entry(&log_path, topic, &new_body, last.timestamp_min)?;
    crate::datalog::append_delete(&log_path, last.offset)?;
    Ok(format!("appended to last entry in {topic}"))
}

fn build_body(text: &str, tags: Option<&str>, source: Option<&str>,
              confidence: Option<f64>, links: Option<&str>) -> String {
    let mut body = String::new();
    if let Some(t) = tags {
        if !t.is_empty() { body.push_str(&format!("[tags: {t}]\n")); }
    }
    if let Some(src) = source { body.push_str(&format!("[source: {src}]\n")); }
    if let Some(c) = confidence {
        if c < 1.0 { body.push_str(&format!("[confidence: {c}]\n")); }
    }
    if let Some(l) = links {
        if !l.is_empty() { body.push_str(&format!("[links: {l}]\n")); }
    }
    body.push_str(text);
    body
}

fn read_text(text: &str) -> Result<String, String> {
    if text == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).map_err(|e| e.to_string())?;
        let trimmed = buf.trim_end();
        if trimmed.is_empty() { return Err("empty stdin".into()); }
        Ok(trimmed.to_string())
    } else {
        Ok(text.to_string())
    }
}

/// Normalize tags: lowercase, trim, singularize, dedupe, sort.
fn normalize_tags(raw: &str) -> String {
    let mut tags: Vec<String> = raw.split(',')
        .map(|t| singularize(t.trim()).to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    tags.sort();
    tags.dedup();
    tags.join(", ")
}

fn singularize(s: &str) -> String {
    let s = s.trim();
    if s.len() <= 3 { return s.to_string(); }
    if s.ends_with("ies") && s.len() > 4 { return format!("{}y", &s[..s.len() - 3]); }
    if s.ends_with("sses") { return s[..s.len() - 2].to_string(); }
    if s.ends_with('s') && !s.ends_with("ss") && !s.ends_with("us") && !s.ends_with("is") {
        return s[..s.len() - 1].to_string();
    }
    s.to_string()
}

/// Auto-detect tags from content prefixes when user provides no explicit tags.
/// Maps known content patterns to canonical tags for better classification.
fn auto_detect_tags(text: &str) -> Option<String> {
    let first = text.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_lowercase())
        .unwrap_or_default();
    let mut tags = Vec::new();
    const PREFIX_TAGS: &[(&str, &str)] = &[
        // gotchas & invariants
        ("gotcha:", "gotcha"),
        ("deploy gotcha:", "gotcha"),
        ("invariant:", "invariant"),
        ("security:", "invariant"),
        // decisions & architecture
        ("decision:", "decision"),
        ("design:", "decision"),
        ("architectural", "decision"),
        ("module:", "module-map"),
        ("overview:", "architecture"),
        // data flow
        ("data flow:", "data-flow"),
        ("flow:", "data-flow"),
        // performance
        ("perf:", "performance"),
        ("benchmark:", "performance"),
        ("hot path:", "performance"),
        // gaps & friction
        ("gap:", "gap"),
        ("missing:", "gap"),
        ("todo:", "gap"),
        ("friction", "gap"),
        // how-to & procedures
        ("how-to:", "how-to"),
        ("impl:", "how-to"),
        ("impl spec:", "how-to"),
        ("shipped", "how-to"),
        ("playbook:", "how-to"),
        // coupling & structure
        ("coupling:", "coupling"),
        ("change impact:", "change-impact"),
        ("transformation:", "coupling"),
        ("pattern:", "pattern"),
        // features & changes
        ("feature:", "how-to"),
        ("bug:", "gotcha"),
        ("fix:", "how-to"),
    ];
    for &(prefix, tag) in PREFIX_TAGS {
        if first.starts_with(prefix) && !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    if tags.is_empty() { None } else { Some(tags.join(", ").to_string()) }
}

fn check_dupe(dir: &Path, topic: &str, new_text: &str) -> Option<String> {
    crate::cache::with_corpus(dir, |cached| {
        // F7: Use cached tf_map for Jaccard similarity instead of body.to_lowercase
        let new_tokens: crate::fxhash::FxHashSet<String> = crate::text::tokenize(new_text)
            .into_iter().filter(|t| t.len() >= 3).collect();
        if new_tokens.len() < 6 { return None; }
        for e in cached.iter().filter(|e| e.topic == topic) {
            let intersection = new_tokens.iter().filter(|t| e.tf_map.contains_key(*t)).count();
            let union = new_tokens.len() + e.tf_map.len() - intersection;
            if union > 0 && intersection as f64 / union as f64 > 0.70 {
                let preview = e.body.trim().lines()
                    .find(|l| !l.starts_with('[') && !l.trim().is_empty())
                    .unwrap_or("").trim();
                let short = if preview.len() > 100 {
                    let mut end = 100;
                    while end > 0 && !preview.is_char_boundary(end) { end -= 1; }
                    format!("{}...", &preview[..end])
                } else { preview.to_string() };
                return Some(short);
            }
        }
        None
    }).ok().flatten()
}

fn suggest_topic(dir: &Path, new_topic: &str) -> Option<String> {
    // F4: Try cached index first, fall back to disk read
    let topics = crate::mcp::with_index(|data| {
        crate::binquery::topic_table(data).ok()
    }).flatten().or_else(|| {
        std::fs::read(dir.join("index.bin")).ok()
            .and_then(|data| crate::binquery::topic_table(&data).ok())
    })?;
    if topics.iter().any(|(_, name, _)| name == new_topic) { return None; }
    let parts: Vec<&str> = new_topic.split('-').collect();
    let similar: Vec<String> = topics.iter()
        .filter(|(_, name, _)| {
            parts.iter().filter(|p| p.len() >= 3 && name.contains(**p)).count() > 0
                && name != new_topic
        })
        .map(|(_, name, _)| name.clone()).collect();
    if similar.is_empty() { return None; }
    Some(format!("new topic. similar: {}", similar.join(", ")))
}

fn validate_links(dir: &Path, links: &str) -> String {
    let mut warnings = Vec::new();
    let _ = crate::cache::with_corpus(dir, |cached| {
        let topics: std::collections::BTreeSet<&str> =
            cached.iter().map(|e| e.topic.as_str()).collect();
        for pair in links.split_whitespace() {
            if let Some((topic, _)) = pair.rsplit_once(':') {
                if !topics.contains(topic) {
                    warnings.push(format!("'{}' not found", topic));
                }
            }
        }
    });
    warnings.join(", ")
}
