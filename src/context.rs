use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

pub fn run(dir: &Path, query: Option<&str>, plain: bool) -> Result<String, String> {
    run_inner(dir, query, plain, false)
}

pub fn run_brief(dir: &Path, query: Option<&str>, plain: bool) -> Result<String, String> {
    run_inner(dir, query, plain, true)
}

fn run_inner(dir: &Path, query: Option<&str>, plain: bool, brief: bool) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    // Query provided → delegate to reconstruct for one-shot briefing
    if let Some(q) = query {
        return crate::reconstruct::run(dir, q);
    }

    // Synthesized meta-briefing for cold starts
    crate::cache::with_corpus(dir, |cached| {
        let mut out = String::new();
        let now_days = crate::time::LocalTime::now().to_days();

        // Activity-weighted topic ranking
        let mut topic_stats: BTreeMap<&str, (usize, i64)> = BTreeMap::new();
        for e in cached {
            let (count, newest) = topic_stats.entry(e.topic.as_str()).or_insert((0, i64::MAX));
            *count += 1;
            let d = e.days_old(now_days);
            if d < *newest { *newest = d; }
        }
        let mut scored: Vec<(&str, usize, i64, f64)> = topic_stats.iter()
            .map(|(&t, &(c, d))| {
                let weight = c as f64 * (1.0 + 1.0 / (1.0 + d as f64 / 7.0));
                (t, c, d, weight)
            }).collect();
        scored.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

        section(&mut out, "Top Topics", plain);
        let limit = if brief { 5 } else { 8 };
        for &(topic, count, days, _) in scored.iter().take(limit) {
            let fresh = freshness(days);
            let _ = writeln!(out, "  {} ({} entries{})", topic, count, fresh);
            if !brief {
                if let Some(e) = cached.iter()
                    .filter(|e| e.topic.as_str() == topic)
                    .min_by_key(|e| e.days_old(now_days))
                {
                    let _ = writeln!(out, "    > {}",
                        crate::text::truncate(e.preview(), 80));
                }
            }
        }
        if scored.len() > limit {
            let _ = writeln!(out, "  ... +{} more topics", scored.len() - limit);
        }

        // Velocity signal
        let today = cached.iter().filter(|e| e.days_old(now_days) == 0).count();
        let week = cached.iter().filter(|e| e.days_old(now_days) <= 7).count();
        let _ = writeln!(out, "\nVelocity: {} entries/7d, {} today ({} total)",
            week, today, cached.len());

        // Cross-topic themes (recent 7d tag frequency)
        if !brief {
            let mut tag_freq: BTreeMap<&str, usize> = BTreeMap::new();
            for e in cached {
                if e.days_old(now_days) > 7 { continue; }
                for t in &e.tags {
                    if t != "raw-data" { *tag_freq.entry(t.as_str()).or_default() += 1; }
                }
            }
            let mut themes: Vec<(&str, usize)> = tag_freq.into_iter().collect();
            themes.sort_by(|a, b| b.1.cmp(&a.1));
            if !themes.is_empty() {
                section(&mut out, "Active Themes (7d)", plain);
                for (tag, count) in themes.iter().take(8) {
                    let _ = writeln!(out, "  {} ({})", tag, count);
                }
            }
        }

        out
    })
}

fn section(out: &mut String, title: &str, plain: bool) {
    if plain {
        let _ = writeln!(out, "\n== {title} ==");
    } else {
        let _ = writeln!(out, "\n\x1b[1;35m== {title} ==\x1b[0m");
    }
}

fn freshness(days: i64) -> &'static str {
    match days { 0 => ", today", 1 => ", 1d", 2..=7 => ", week", _ => "" }
}
