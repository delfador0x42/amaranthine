// Comprehensive benchmark for amaranthine v5.3 post-optimization
// Measures: iter_live, tokenize, rebuild, search_scored (3 queries),
//           brief/medium/full/topics/count, reconstruct, binquery raw,
//           store→search round-trip

use std::time::Instant;
use std::path::Path;

fn main() {
    let dir = amaranthine::config::resolve_dir(None);
    let log_path = dir.join("data.log");
    let index_path = dir.join("index.bin");
    let log_size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
    let idx_size = std::fs::metadata(&index_path).map(|m| m.len()).unwrap_or(0);

    // Warm cache once
    let _ = amaranthine::cache::with_corpus(&dir, |c| c.len());
    let entry_count = amaranthine::cache::with_corpus(&dir, |c| c.len()).unwrap_or(0);

    eprintln!("=== AMARANTHINE BENCHMARK ===");
    eprintln!("dir: {}", dir.display());
    eprintln!("data.log: {} bytes, index.bin: {} bytes, entries: {}", log_size, idx_size, entry_count);
    eprintln!();

    let index_data = std::fs::read(&index_path).ok();
    let idx = index_data.as_deref();
    let filter_none = amaranthine::score::Filter::none();

    // --- PARSE PATH ---
    eprintln!("--- PARSE PATH ---");

    bench("iter_live (parse data.log)", 20, || {
        let _ = amaranthine::datalog::iter_live(&log_path).unwrap();
    });

    bench("iter_live + tokenize(all)", 20, || {
        let entries = amaranthine::datalog::iter_live(&log_path).unwrap();
        for e in &entries {
            let _ = amaranthine::text::tokenize(&e.body);
        }
    });

    bench("tokenize(single 100-word)", 50, || {
        let _ = amaranthine::text::tokenize(
            "The iris endpoint security scanner detects nation-state malware using \
             behavioral analysis and Shannon entropy calculations on process memory \
             regions. Each scanner implements a protocol for reporting findings to \
             the central detection engine via XPC connections. Performance metrics \
             are tracked per-scan including latency percentiles and false positive \
             rates across different threat categories and severity levels."
        );
    });

    // --- WRITE PATH ---
    eprintln!("\n--- WRITE PATH ---");

    // Cold rebuild (invalidate cache first)
    bench("rebuild() cold cache", 5, || {
        amaranthine::cache::invalidate();
        let _ = amaranthine::inverted::rebuild(&dir);
    });

    // Warm rebuild
    let _ = amaranthine::cache::with_corpus(&dir, |_| ());
    bench("rebuild() warm cache", 10, || {
        let _ = amaranthine::inverted::rebuild(&dir);
    });

    bench("fs::read(index.bin)", 50, || {
        let _ = std::fs::read(&index_path).unwrap();
    });

    // --- READ PATH (raw index) ---
    eprintln!("\n--- READ PATH (raw index) ---");

    if let Some(data) = idx {
        bench("binquery::search_v2(single)", 200, || {
            let _ = amaranthine::binquery::search_v2(data, "performance", 10);
        });
        bench("binquery::search_v2(multi)", 200, || {
            let _ = amaranthine::binquery::search_v2(data, "iris endpoint security", 10);
        });
        bench("50 sequential binquery calls", 20, || {
            for _ in 0..50 {
                let _ = amaranthine::binquery::search_v2(data, "performance", 10);
            }
        });
    }

    // --- READ PATH (search_scored) ---
    eprintln!("\n--- READ PATH (search_scored) ---");

    let terms_single = amaranthine::text::query_terms("performance");
    let terms_multi = amaranthine::text::query_terms("iris endpoint security");
    let terms_miss = amaranthine::text::query_terms("nonexistent xyzzy plugh");

    bench("search_scored(single-term, index)", 50, || {
        let _ = amaranthine::score::search_scored(&dir, &terms_single, &filter_none, Some(10), idx, true).unwrap();
    });

    bench("search_scored(multi-AND, index)", 50, || {
        let _ = amaranthine::score::search_scored(&dir, &terms_multi, &filter_none, Some(10), idx, true).unwrap();
    });

    bench("search_scored(OR fallback)", 50, || {
        let _ = amaranthine::score::search_scored(&dir, &terms_miss, &filter_none, Some(10), idx, true).unwrap();
    });

    // Light hydration (brief/medium) vs full hydration
    bench("search_scored(single, full_body=true)", 50, || {
        let _ = amaranthine::score::search_scored(&dir, &terms_single, &filter_none, Some(10), idx, true).unwrap();
    });

    bench("search_scored(single, full_body=false)", 50, || {
        let _ = amaranthine::score::search_scored(&dir, &terms_single, &filter_none, Some(10), idx, false).unwrap();
    });

    // --- READ PATH (search formatters) ---
    eprintln!("\n--- READ PATH (search formatters) ---");

    bench("search::run_brief(single)", 50, || {
        let _ = amaranthine::search::run_brief(&dir, "performance", Some(10), &filter_none, idx).unwrap();
    });

    bench("search::run_medium(single)", 50, || {
        let _ = amaranthine::search::run_medium(&dir, "performance", Some(10), &filter_none, idx).unwrap();
    });

    bench("search::run(single, full)", 50, || {
        let _ = amaranthine::search::run(&dir, "performance", true, Some(10), &filter_none, idx).unwrap();
    });

    bench("search::run_topics", 50, || {
        let _ = amaranthine::search::run_topics(&dir, "performance", &filter_none).unwrap();
    });

    bench("search::count", 50, || {
        let _ = amaranthine::search::count(&dir, "performance", &filter_none).unwrap();
    });

    // --- RECONSTRUCT ---
    eprintln!("\n--- RECONSTRUCT ---");

    bench("reconstruct::run('iris')", 20, || {
        let _ = amaranthine::reconstruct::run(&dir, "iris").unwrap();
    });

    // --- CORPUS PATH ---
    eprintln!("\n--- CORPUS PATH ---");

    bench("with_corpus warm (no-op)", 50, || {
        let _ = amaranthine::cache::with_corpus(&dir, |c| c.len());
    });

    bench("with_corpus cold + reload", 5, || {
        amaranthine::cache::invalidate();
        let _ = amaranthine::cache::with_corpus(&dir, |c| c.len());
    });

    eprintln!("\n=== DONE ===");
}

fn bench(name: &str, iters: usize, mut f: impl FnMut()) {
    // Warmup
    f();

    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        f();
        times.push(start.elapsed());
    }
    times.sort();
    let p50 = times[iters / 2];
    let p99 = times[iters * 99 / 100];
    let min = times[0];
    eprintln!("  {name:48} min={min:>10.1?}  p50={p50:>10.1?}  p99={p99:>10.1?}");
}
