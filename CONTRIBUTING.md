# Contributing to amaranthine

## Building

```bash
cargo build --release
```

The release binary lands in `target/release/amaranthine`. Zero external dependencies —
everything (JSON parser, hasher, binary index, date math) is hand-rolled.

To install locally and register with Claude Code:

```bash
make install
```

To deploy after changes without re-running install:

```bash
make deploy    # copies binary, codesigns, prints reminder
```

Or if you're running the MCP server already, hot-reload from within Claude Code:

```
Use the _reload tool
```

This re-execs the server process with the new binary (no restart needed).

## Project Structure

44 Rust files, ~9,800 lines. One file, one job.

### Core Data Layer

| File | Lines | What |
|------|-------|------|
| `datalog.rs` | ~210 | Append-only data log: read, write, compact, migrate. Single source of truth. |
| `format.rs` | ~90 | Binary index on-disk structs. All `#[repr(C, packed)]` for zero-copy access. |
| `inverted.rs` | ~420 | Index builder: reads data.log, produces index.bin with BM25-ready postings. |
| `binquery.rs` | ~570 | Index reader: 3-phase deferred snippets, ~200ns queries. |
| `cache.rs` | ~190 | In-memory corpus cache with mtime invalidation. Pre-tokenized entries. |

### Search & Scoring

| File | Lines | What |
|------|-------|------|
| `score.rs` | ~350 | BM25 scoring engine. AND-to-OR fallback, topic/tag boost, confidence weighting. |
| `search.rs` | ~180 | Search output formatting: full, medium, brief, count, topics, grouped. |
| `text.rs` | ~300 | Unified tokenizer: ASCII fast path, CamelCase/snake_case split, tag parser. |

### Write Path

| File | Lines | What |
|------|-------|------|
| `store.rs` | ~285 | Entry creation with Jaccard dedup, auto-tags, confidence, links, source. |
| `edit.rs` | ~175 | Entry modification: update, append, tag operations. All append+tombstone. |
| `delete.rs` | ~96 | Entry/topic deletion via tombstone records. |

### Compression & Synthesis

| File | Lines | What |
|------|-------|------|
| `compress.rs` | ~280 | Cross-topic dedup + Jaccard similarity chains + temporal chains. |
| `briefing.rs` | ~570 | 3-pass category classification, format_summary, body-keyword rescue. |
| `reconstruct.rs` | ~200 | One-shot synthesis: topic matching + link following + glob patterns. |

### Codebase Analysis

| File | Lines | What |
|------|-------|------|
| `callgraph.rs` | ~175 | Trace function callers/callees with configurable depth. |
| `codepath.rs` | ~180 | Find access sites for a pattern with context lines. |
| `reverse.rs` | ~410 | Architecture mapping: module relationships, exports, coupling. |
| `crash.rs` | ~230 | Stack frame parsing + crash pattern matching to source. |
| `perf.rs` | ~220 | Callgraph + allocation/lock/I/O antipattern detection. |

### MCP Server

| File | Lines | What |
|------|-------|------|
| `mcp.rs` | ~430 | JSON-RPC stdio loop, index management, audit on reload. |
| `mcp/tools.rs` | ~230 | 26 tool schema definitions. |
| `mcp/dispatch.rs` | ~500 | Tool call routing, argument extraction, filter building. |

### Browse & Stats

| File | Lines | What |
|------|-------|------|
| `topics.rs` | ~135 | Topic listing, recent entries, preview formatting. |
| `context.rs` | ~97 | Session briefing: activity-weighted topics + velocity. |
| `digest.rs` | ~32 | One-bullet-per-entry summaries. |
| `stats.rs` | ~220 | Statistics, tag listing, index health. |
| `export.rs` | ~80 | JSON export/import with timestamp preservation. |
| `xref.rs` | ~95 | Cross-reference finder. |
| `depgraph.rs` | ~170 | Topic dependency graph with glob filtering. |

### Infrastructure

| File | Lines | What |
|------|-------|------|
| `json.rs` | ~405 | Recursive descent JSON parser + fast-path strings + escape_into. |
| `fxhash.rs` | ~82 | Word-at-a-time multiply-rotate hasher, ~3ns/op. |
| `intern.rs` | ~77 | `InternedStr`: Arc<str> newtype. O(1) clone for topic names. |
| `time.rs` | ~205 | Date math: minutes-since-epoch, relative dates, zero-format. |
| `config.rs` | ~195 | Directory resolution, path sanitization, source path resolution. |
| `lock.rs` | ~31 | Unix `flock()` for write serialization. |
| `compact.rs` | ~115 | Duplicate detection within topics. |
| `prune.rs` | ~40 | Stale topic flagging. |
| `migrate.rs` | ~40 | Timestamp backfill for legacy entries. |

### Entry Points

| File | Lines | What |
|------|-------|------|
| `main.rs` | ~280 | CLI entry: arg parsing, subcommand dispatch, hook routing. |
| `lib.rs` | ~200 | Library root: module declarations + C FFI exports. |
| `cffi.rs` | ~125 | C FFI zero-alloc query path with generation counter. |
| `hook.rs` | ~500 | Hook handlers: mmap ambient, post-build, stop, subagent-start. |
| `sock.rs` | ~225 | Unix domain socket listener for hook queries. |
| `install.rs` | ~195 | Installer: binary copy, codesign, MCP config, hooks. |

## Architecture: Three Access Tiers

```
Tier 1: C FFI (~200ns)
  cffi.rs -> binquery.rs -> format.rs structs on index.bin
  Zero allocation. Generation counter skips array zeroing.
  Hook mmap path bypasses socket for sub-millisecond context injection.

Tier 2: MCP Server (~5ms)
  mcp.rs -> dispatch.rs -> score.rs/search.rs -> cache.rs -> datalog.rs
  JSON-RPC over stdio. 26 tools. In-process, no IPC to data.
  Used by the agent during sessions.

Tier 3: CLI (~5ms)
  main.rs -> same paths as MCP
  Direct invocation. Used for manual queries and scripting.
```

## Data Flow

### Write

```
store(topic, text, tags?, confidence?, links?)
  -> build_body(): prepend metadata lines ([tags:], [source:], [confidence:], [links:])
  -> dupe check: 6-char word Jaccard at 90% threshold
  -> datalog::append_entry(): write to data.log
  -> inverted::ensure_index(): full rebuild of index.bin
  -> cache::invalidate(): clear corpus cache
```

### Read

```
search(query, detail?, filter?)
  -> cache::with_corpus(): load entries (warm: 0us, cold: ~5ms)
  -> score::search_scored(): BM25 with topic boost + tag boost + confidence
  -> search::run_*(): format output per detail level
```

### Brief (One-Shot Reconstruction)

```
brief(query?)
  -> identify topics by name match or glob pattern
  -> collect related entries via token matching
  -> follow narrative links one level deep
  -> compress: Jaccard dedup + temporal chains + supersession
  -> briefing: 3-pass classify + format_summary
```

### Index

```
index.bin layout:
  [Header 72B][TermTable][Postings][EntryMeta][Snippets][TopicTable][TopicNames][SourcePool][XrefTable]

  TermTable: open-addressing hash map (FNV-1a, power-of-two capacity)
  Postings: pre-baked IDF, per-entry TF
  EntryMeta: topic_id, word_count, snippet, date, source, confidence, log_offset
```

## Entry Metadata Format

Entries are plain text with optional metadata prefix lines:

```
[tags: rust, ffi, performance]
[source: src/cache.rs:42]
[confidence: 0.8]
[links: iris-engine:42 iris-project:7]
This is the actual entry content.
Multiple lines are fine.
```

- **tags**: comma-separated, stored lowercase, used for filtering and scoring
- **source**: file path for staleness detection (relative to project root)
- **confidence**: 0.0-1.0, affects BM25 ranking (default 1.0 if omitted)
- **links**: space-separated `topic:index` pairs for narrative connections

## How to Add a New MCP Tool

1. **Define the schema** in `mcp/tools.rs`:
   ```rust
   tool("my_tool", "Description of what it does.",
       &["required_param"],
       &[("required_param", "string", "What this param is"),
         ("optional_param", "string", "Optional description")])
   ```

2. **Add the dispatch arm** in `mcp/dispatch.rs`:
   ```rust
   "my_tool" => {
       let param = arg_str(args, "required_param");
       // ... implementation ...
       Ok(result_string)
   }
   ```

3. **If the tool writes data**, call `super::after_write(dir, &topic)` after the operation. This rebuilds the index and invalidates the cache.

4. **Build and deploy**:
   ```bash
   cargo build --release
   ```
   Then `_reload` from within Claude Code, or `make deploy`.

## Key Invariants

These are load-bearing. Violating them corrupts data or crashes.

- **data.log is append-only.** Never modify in place. All mutations append new records.
- **Entry header is exactly 12 bytes** (type:1 + topic_len:1 + body_len:4 + ts_min:4 + pad:2). The 2 pad bytes ensure alignment. Changing this breaks all existing data.log files.
- **index.bin is rebuilt from scratch** after every write. Never incrementally updated.
- **Term hash 0 is the empty slot sentinel.** `hash_term()` returns 1 if FNV-1a computes 0.
- **All format.rs structs are `#[repr(C, packed)]`** — reads use `ptr::read_unaligned` to avoid SIGBUS on ARM64.
- **Topic names max 255 bytes** (u8 length field).
- **Timestamps are i32 minutes-since-epoch.** Good until ~4085 CE.
- **JSON `Obj` is `Vec<(String, Value)>`**, not HashMap — preserves insertion order.

## Change Impact Guide

Changing these files has cascading effects:

| File | Impact |
|------|--------|
| `format.rs` | Must bump VERSION. Update `inverted.rs` (builder), `binquery.rs` (reader), `cffi.rs` (FFI). Not backward compatible. |
| `datalog.rs` | Entry record format change breaks ALL existing data.log files. No migration path for header changes. |
| `json.rs` | Used by mcp.rs, dispatch.rs, export.rs, install.rs, tools.rs, main.rs. `Value` enum changes cascade everywhere. |
| `cache.rs` | `CachedEntry` struct used by score.rs, search.rs, reconstruct.rs, topics.rs, stats.rs, digest.rs, export.rs, xref.rs, depgraph.rs. Adding fields requires updating `with_corpus()`. |
| `text.rs` | `tokenize()` and `query_terms()` affect both search paths (corpus BM25 and binary index). Changes alter what matches what. |
| `score.rs` | `Filter` struct used by dispatch.rs, context.rs, reconstruct.rs. Adding a filter field requires updating `build_filter()` in dispatch.rs. |
| `briefing.rs` | Categories and classification logic affect all `brief` output. Changes here change the mental model agents build. |

## Testing

There's no test suite yet (the codebase is tested via real usage). To verify changes:

```bash
# Build
cargo build --release

# Smoke test: store, search, brief
./target/release/amaranthine store test-topic "test entry" --tags test
./target/release/amaranthine search "test"
./target/release/amaranthine call brief query="test"

# Clean up
./target/release/amaranthine call delete topic="test-topic" all="true"
```

## Style

- Zero dependencies. If you need something, build it.
- One file, one job. Target 100 lines, max 300 (analysis files may go to ~600).
- Simple over clever. If it needs a comment to explain a trick, simplify it.
- `#[inline]` on functions called cross-module in hot paths.
- `with_capacity` on all hot-path allocations.
- Topics and tags are always lowercase (enforced at write time).
- Zero `format!()` in hot paths — use `push_str` with pre-sized buffers.
- FxHashSet/FxHashMap for all internal data structures (no DoS concern).
