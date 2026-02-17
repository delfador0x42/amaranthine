# Amaranthine — Persistent Knowledge Base

## What This Does
Fast, local CLI + library for storing and retrieving development knowledge
across AI-assisted coding sessions. Three access paths: MCP server (~5ms),
in-memory cache (~25μs), C FFI dylib (~1μs). Plain markdown files, zero
dependencies, frictionless capture. No cloud, no database, no runtime deps.

## Why This Design
Each AI session starts with ~100 lines of context for a 50K-line project.
Topic files help but require knowing which file to read — circular when
you don't know what you don't know. amaranthine makes the right thing easy:
one command to store, one to search, one to orient at session start.

## Performance Architecture
Three tiers, each eliminating a layer of overhead:
1. **MCP server** (~5ms): JSON-RPC over stdio, full filtering/formatting
2. **In-memory cache** (~25μs): mtime-invalidated corpus, skips file I/O
3. **C FFI dylib** (~1μs): direct in-process query, no IPC
   - `libamaranthine.dylib` (422KB) with 7-function C API
   - Binary inverted index: BM25 scoring, FNV-1a hashing, open addressing
   - 271 entries, 9138 terms, ~458KB index fits in L2 cache
   - Benchmark: 986ns single-term, 1719ns multi-term, 807ns stale check

## Data Flow
`store` → append timestamped entry to `<topic>.md` → invalidate cache → rebuild index
`search` → cache-backed BM25 with AND→OR fallback, topic/tag/date filtering
`index_search` → binary index query via mmap'd `index.bin` (~200ns computation)
`serve` → MCP server over stdio (JSON-RPC, 34 tools, in-process dispatch)
C FFI → `amr_open` → `amr_search` → `amr_free_str` → `amr_close`

## Decisions Made
- Plain markdown: human-readable, git-trackable, grep-able
- BM25 ranking: CamelCase/snake_case aware, header boost=2.0
- Zero dependencies: hand-rolled JSON, libc FFI, binary index
- Binary index format: [Header][TermTable][PostingLists][EntryMeta][SnippetPool]
- `#[repr(C, packed)]` structs for zero-copy mmap access
- FNV-1a 64-bit hash: fast, good distribution, no deps
- Cache: `Mutex<Option<CorpusCache>>` with mtime invalidation per file
- lib.rs + main.rs split: library crate (modules + C FFI) + binary consumer
- `strip = "debuginfo"`: preserves C symbol exports while removing debug bloat
- Atomic writes: tmp + fsync + rename prevents corruption on crash

## Key Files
- `src/lib.rs` — library root: pub modules + C FFI (amr_open/search/close)
- `src/main.rs` — CLI entry, manual arg parsing
- `src/binquery.rs` — binary index query engine (~200ns per query)
- `src/inverted.rs` — binary index builder (FNV-1a, open addressing, BM25)
- `src/cache.rs` — in-memory corpus cache with mtime invalidation
- `src/search.rs` — BM25 search + filtering + multiple output formats
- `src/mcp.rs` — MCP server: 34 tools, cache/index auto-maintenance
- `src/json.rs` — recursive descent JSON parser + pretty printer
- `include/amaranthine.h` — C header for FFI consumers
- `tests/bench.c` — C benchmark harness
