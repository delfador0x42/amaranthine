// Burst workload for profiling: runs hot paths in a tight loop.
// Use: `sample <pid> 3` while this is running.

use std::time::Instant;

fn main() {
    let dir = amaranthine::config::resolve_dir(None);
    let index_data = std::fs::read(dir.join("index.bin")).ok();
    let idx = index_data.as_deref();
    let filter = amaranthine::score::Filter::none();

    // Warm cache
    let _ = amaranthine::cache::with_corpus(&dir, |_| ());

    eprintln!("starting burst workload (pid {})...", std::process::id());
    eprintln!("run: sample {} 3 -mayDie", std::process::id());
    // Brief pause to allow attaching profiler
    std::thread::sleep(std::time::Duration::from_secs(2));

    let start = Instant::now();
    let mut iters = 0u64;

    // Phase 1: 200 index searches (raw binquery)
    if let Some(data) = idx {
        for _ in 0..200 {
            let _ = amaranthine::binquery::search_v2(data, "performance", 10);
            let _ = amaranthine::binquery::search_v2(data, "iris endpoint security", 10);
            iters += 2;
        }
    }

    // Phase 2: 100 full search_scored calls (mixed queries)
    let terms_single = amaranthine::text::query_terms("performance");
    let terms_multi = amaranthine::text::query_terms("iris endpoint security");
    for _ in 0..50 {
        let _ = amaranthine::score::search_scored(&dir, &terms_single, &filter, Some(10), idx, true);
        let _ = amaranthine::score::search_scored(&dir, &terms_multi, &filter, Some(10), idx, true);
        iters += 2;
    }

    // Phase 3: 50 brief/medium (light hydration path)
    for _ in 0..50 {
        let _ = amaranthine::search::run_brief(&dir, "performance", Some(10), &filter, idx);
        let _ = amaranthine::search::run_medium(&dir, "security", Some(10), &filter, idx);
        iters += 2;
    }

    // Phase 4: 50 search_topics + count (corpus path, post clone-elimination)
    for _ in 0..50 {
        let _ = amaranthine::search::run_topics(&dir, "performance", &filter);
        let _ = amaranthine::search::count(&dir, "iris", &filter);
        iters += 2;
    }

    // Phase 5: 15 rebuilds (warm cache)
    for _ in 0..15 {
        let _ = amaranthine::inverted::rebuild(&dir);
        iters += 1;
    }

    // Phase 6: 20 reconstructs
    for _ in 0..20 {
        let _ = amaranthine::reconstruct::run(&dir, "iris");
        iters += 1;
    }

    // Phase 7: 50 corpus loads (cold, to stress cache miss path)
    for _ in 0..10 {
        amaranthine::cache::invalidate();
        let _ = amaranthine::cache::with_corpus(&dir, |c| c.len());
        iters += 1;
    }

    let elapsed = start.elapsed();
    eprintln!("done: {iters} operations in {elapsed:.1?}");
}
