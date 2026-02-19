//! Performance benchmark for amaranthine hot paths.
//! Run: cargo run --release --bin perf_bench

use std::path::Path;
use std::time::Instant;

fn percentile(mut times: Vec<f64>, p: f64) -> f64 {
    if times.is_empty() { return 0.0; }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((p / 100.0) * (times.len() - 1) as f64).round() as usize;
    times[idx.min(times.len() - 1)]
}

fn bench<F: FnMut()>(name: &str, iters: usize, mut f: F) -> Vec<f64> {
    // Warmup
    for _ in 0..3 { f(); }
    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        f();
        times.push(start.elapsed().as_secs_f64() * 1_000_000.0); // microseconds
    }
    let p50 = percentile(times.clone(), 50.0);
    let p99 = percentile(times.clone(), 99.0);
    let unit = if p50 > 1000.0 { "ms" } else { "µs" };
    let scale = if p50 > 1000.0 { 1000.0 } else { 1.0 };
    eprintln!("  {name:42} p50={:>8.1}{unit}  p99={:>8.1}{unit}", p50 / scale, p99 / scale);
    times
}

fn main() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let dir = Path::new(&home).join(".amaranthine");
    let dir = dir.as_path();
    let log_path = dir.join("data.log");

    eprintln!("=== Amaranthine Performance Benchmark ===");
    eprintln!("  dir: {}", dir.display());
    let log_size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
    eprintln!("  data.log: {} KB", log_size / 1024);

    // --- iter_live ---
    let entries = amaranthine::datalog::iter_live(&log_path).unwrap();
    let entry_count = entries.len();
    eprintln!("  entries: {entry_count}");
    eprintln!();

    eprintln!("--- PARSE PATH ---");
    bench("iter_live (parse data.log)", 20, || {
        let _ = amaranthine::datalog::iter_live(&log_path).unwrap();
    });

    // iter_live + tokenize all
    bench("iter_live + tokenize(all entries)", 20, || {
        let entries = amaranthine::datalog::iter_live(&log_path).unwrap();
        for e in &entries { let _ = amaranthine::text::tokenize(&e.body); }
    });

    // tokenize alone (single entry)
    let sample = &entries[0].body;
    bench("tokenize(single entry)", 100, || {
        let _ = amaranthine::text::tokenize(sample);
    });

    eprintln!();
    eprintln!("--- WRITE PATH ---");

    // Rebuild (cold: invalidate cache first)
    bench("rebuild() cold cache", 10, || {
        amaranthine::cache::invalidate();
        let _ = amaranthine::inverted::rebuild(dir).unwrap();
    });

    // Rebuild (warm: cache already populated)
    // First, warm the cache
    let _ = amaranthine::inverted::rebuild(dir);
    bench("rebuild() warm cache", 20, || {
        let _ = amaranthine::inverted::rebuild(dir).unwrap();
    });

    // fs::read(index.bin)
    let index_path = dir.join("index.bin");
    let index_size = std::fs::metadata(&index_path).map(|m| m.len()).unwrap_or(0);
    eprintln!("  index.bin: {} KB", index_size / 1024);
    bench("fs::read(index.bin)", 50, || {
        let _ = std::fs::read(&index_path).unwrap();
    });

    eprintln!();
    eprintln!("--- READ PATH (index) ---");

    let index_data = std::fs::read(&index_path).unwrap();

    // binquery::search_v2 (raw index path)
    bench("binquery::search_v2('performance')", 50, || {
        let _ = amaranthine::binquery::search_v2(&index_data, "performance", 10).unwrap();
    });

    bench("binquery::search_v2('iris endpoint')", 50, || {
        let _ = amaranthine::binquery::search_v2(&index_data, "iris endpoint security", 10).unwrap();
    });

    bench("binquery::search_v2 OR('nonexist xyz')", 50, || {
        let _ = amaranthine::binquery::search_v2_or(
            &index_data, "nonexistent xyzzy plugh",
            &amaranthine::binquery::FilterPred::none(), 10
        ).unwrap();
    });

    eprintln!();
    eprintln!("--- READ PATH (score.rs full path) ---");

    let filter = amaranthine::score::Filter::none();

    // search_scored — single term (via index)
    bench("search_scored('performance', index)", 20, || {
        let terms = amaranthine::text::query_terms("performance");
        let _ = amaranthine::score::search_scored(dir, &terms, &filter, Some(10), Some(&index_data)).unwrap();
    });

    // search_scored — multi-term AND
    bench("search_scored('iris endpoint', AND)", 20, || {
        let terms = amaranthine::text::query_terms("iris endpoint security scanner");
        let _ = amaranthine::score::search_scored(dir, &terms, &filter, Some(10), Some(&index_data)).unwrap();
    });

    // search_scored — OR fallback
    bench("search_scored('nonexist xyz', OR fb)", 20, || {
        let terms = amaranthine::text::query_terms("nonexistent xyzzy plugh");
        let _ = amaranthine::score::search_scored(dir, &terms, &filter, Some(10), Some(&index_data)).unwrap();
    });

    eprintln!();
    eprintln!("--- READ PATH (corpus) ---");

    // load_corpus (warm cache)
    bench("load_corpus(warm cache)", 20, || {
        let _ = amaranthine::score::load_corpus(dir, &filter).unwrap();
    });

    // load_corpus (cold)
    bench("load_corpus(cold cache)", 10, || {
        amaranthine::cache::invalidate();
        let _ = amaranthine::score::load_corpus(dir, &filter).unwrap();
    });

    // search_topics (corpus path)
    bench("search::run_topics('performance')", 20, || {
        let _ = amaranthine::search::run_topics(dir, "performance", &filter).unwrap();
    });

    // search::count (corpus path)
    bench("search::count('performance')", 20, || {
        let _ = amaranthine::search::count(dir, "performance", &filter).unwrap();
    });

    eprintln!();
    eprintln!("--- RECONSTRUCT ---");

    bench("reconstruct::run('iris')", 10, || {
        let _ = amaranthine::reconstruct::run(dir, "iris").unwrap();
    });

    eprintln!();
    eprintln!("--- BURST (50 sequential index_search) ---");

    let start = Instant::now();
    let queries = ["performance", "iris", "scanner", "engine", "network",
        "endpoint", "proxy", "shared", "build", "gotcha"];
    for _ in 0..5 {
        for q in &queries {
            let _ = amaranthine::binquery::search(&index_data, q, 10).unwrap();
        }
    }
    let burst_us = start.elapsed().as_secs_f64() * 1_000_000.0;
    eprintln!("  50 sequential searches: {:.0}µs total ({:.1}µs/call)", burst_us, burst_us / 50.0);

    eprintln!();
    eprintln!("--- HASH ---");
    bench("hash_term('performance')", 200, || {
        let _ = amaranthine::format::hash_term("performance");
    });

    bench("hash_term('iris')", 200, || {
        let _ = amaranthine::format::hash_term("iris");
    });

    eprintln!();
    eprintln!("=== DONE ===");
}
