//! Profiling target: burst operations for sample/instruments.
//! Run: cargo run --release --bin profile_burst
//! Profile: sample profile_burst 5 -file profile_out.txt

use std::path::Path;

fn main() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let dir_path = format!("{home}/.amaranthine");
    let dir = Path::new(&dir_path);
    let index_data = std::fs::read(dir.join("index.bin")).unwrap();
    let filter = amaranthine::score::Filter::none();
    let queries = ["performance", "iris scanner", "engine network",
        "endpoint proxy", "build gotcha", "rust ffi", "xnu kernel",
        "detection scanner", "shell deobfuscator", "persistence monitor"];

    eprintln!("Starting burst: 100 search_scored + 15 rebuilds + 50 corpus loads");

    // Phase 1: 100 search_scored via index
    for round in 0..10 {
        for q in &queries {
            let terms = amaranthine::text::query_terms(q);
            let _ = amaranthine::score::search_scored(
                dir, &terms, &filter, Some(10), Some(&index_data)
            );
        }
        if round % 5 == 0 { eprint!("."); }
    }
    eprintln!(" searches done");

    // Phase 2: 15 rebuild (warm cache)
    for _ in 0..15 {
        let _ = amaranthine::inverted::rebuild(dir);
    }
    eprintln!(" rebuilds done");

    // Phase 3: 50 load_corpus (warm cache, cloning is the bottleneck)
    for _ in 0..50 {
        let _ = amaranthine::score::load_corpus(dir, &filter);
    }
    eprintln!(" corpus loads done");

    // Phase 4: 20 reconstruct
    for _ in 0..20 {
        let _ = amaranthine::reconstruct::run(dir, "iris");
    }
    eprintln!(" reconstructs done");

    // Phase 5: 200 raw index searches
    for _ in 0..20 {
        for q in &queries {
            let _ = amaranthine::binquery::search(&index_data, q, 10);
        }
    }
    eprintln!(" index searches done");

    eprintln!("Burst complete.");
}
