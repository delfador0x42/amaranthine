# Amaranthine — Design

## What This Does

Fast, local knowledge base for AI coding agents. Two access paths: MCP server (~5ms)
and C FFI (~200ns). Single append-only data log, binary inverted index, BM25 search,
26 MCP tools, zero dependencies. No cloud, no database, no runtime deps.

## Why This Design

Each AI session starts with ~100 lines of context for a 50K-line project.
Topic files help but require knowing which file to read — circular when
you don't know what you don't know. amaranthine makes the right thing easy:
one command to store, one to search, one to reconstruct full understanding.

## Storage Architecture

Single-file append-only data log (`data.log`) + binary inverted index (`index.bin`):

- **Write path**: store -> append EntryRecord to data.log -> rebuild index.bin
- **Read path**: search/brief/stats -> BM25 over data.log entries
- **Fast path**: C FFI -> query index.bin directly (~200ns, zero alloc)
- **Delete**: append DeleteRecord (tombstone) referencing target's byte offset
- **Edit**: append new EntryRecord + DeleteRecord for old version
- Topics are virtual metadata — no per-topic files, no filesystem overhead

### data.log format

Magic `b'AMRL'`. Two record types:
- Entry: `[0x01, topic_len:u8, body_len:u32, ts_min:i32, pad:2, topic, body]`
- Delete: `[0x02, pad:3, offset:u32]` — tombstones an entry by byte offset

### index.bin format

Magic `b'AMRN'` v2. Sections: Header -> TermTable -> Postings -> EntryMeta ->
Snippets -> TopicTable -> TopicNames -> SourcePool -> XrefTable.
All `#[repr(C, packed)]` structs for zero-copy mmap access.

### Entry metadata

Entries can carry structured metadata as prefix lines in the body:
- `[tags: rust, ffi]` — comma-separated tags for filtering and scoring
- `[source: src/main.rs:42]` — source file provenance for staleness detection
- `[confidence: 0.8]` — 0.0-1.0, affects search ranking (default 1.0)
- `[links: topic:idx topic:idx]` — narrative links to other entries
- `[type: ...]`, `[tier: ...]`, `[modified]` — informational annotations

## Search

Unified tokenizer (`text::tokenize`): byte-level ASCII fast path, split on
non-alphanumeric, expand CamelCase/snake_case, lowercase, min 2 chars.
Falls back to Unicode for non-ASCII content (<1% of entries).

- BM25 scoring with K1=1.2, B=0.75
- Topic-name boost: 1.5x multiplicative for entries in matching topics
- Tag-aware scoring: +30% per query term matching entry tags
- Confidence-weighted: entries with explicit confidence < 1.0 score lower
- AND->OR fallback: multi-word queries retry as OR when AND returns 0 results
- Conservative SEARCH_STOP_WORDS (pure function words, no technical terms)
- FxHash (`fxhash.rs`) for all internal HashMap/HashSet (~3ns vs SipHash ~20ns)
- Single `search` tool with `detail` param: full/medium/brief/count/topics/grouped/index

## Briefing (One-Shot Reconstruction)

`brief` builds compressed briefings from stored knowledge:

1. Identifies primary topics (name contains query, or glob match like `iris-*`)
2. Collects related entries via token matching
3. Follows narrative links one level deep
4. Compresses via cross-topic dedup + temporal chains (Jaccard similarity)
5. Classifies entries into categories (architecture, data flow, gotchas, gaps, etc.)
6. Outputs hierarchical, tag-categorized briefing with freshness weighting

Three output tiers: `summary` (~15 lines), `scan` (category one-liners), `full` (complete entries).
Without a query, produces a meta-briefing (activity-weighted topics + velocity).

## Compression Engine

`compress.rs` reduces redundancy across collected entries:
- **Cross-topic dedup**: Jaccard similarity >40% token overlap, single-linkage clustering
- **Temporal chains**: entries within 48h buckets with overlapping topics get chained
- **Supersession**: newer entries that cover the same ground mark older ones `[SUPERSEDED]`
- **Freshness weighting**: 7-day half-life boost; architecture/invariant entries are exempt

`briefing.rs` classifies compressed entries into display categories:
- 3-pass classification: structural -> static (tag + keyword + prefix) -> dynamic -> untagged
- Body-keyword rescue: scans content lines to rescue untagged entries into proper categories
- GAPS category: catches gap/friction/todo/missing entries separately

## Codebase Analysis (trace)

`trace` provides 7 analysis modes for inspecting codebases:

| Mode | What |
|------|------|
| `callgraph` | Trace function callers/callees with configurable depth |
| `codepath` | Find all access sites for a pattern with context |
| `reverse` | Map architecture — module relationships, exports, coupling |
| `core` | BFS reachability from entry points — find core vs dead code |
| `simplify` | Jaccard similarity between files — find duplicates/thin wrappers |
| `crash` | Parse stack frames + match crash patterns to source |
| `perf` | Callgraph + antipattern detection (allocations, locks, I/O in hot paths) |

## Hooks

Four Claude Code hooks in `hook.rs`, dispatched via `amaranthine hook <type>`:
- **ambient** (PreToolUse): mmap-reads binary index, queries on file stem before Read/Edit/Write
- **post-build** (PostToolUse Bash): matches build commands, reminds to store findings
- **stop** (Stop): debounced 120s reminder to persist findings
- **subagent-start** (SubagentStart): dynamic topic list from index

The ambient hook uses direct mmap(2) on index.bin — zero socket overhead, sub-millisecond.
Installed globally to `~/.claude/settings.json` by `amaranthine install`.

## Staleness Detection

Entries with `[source: path/to/file:line]` metadata track source file provenance.
- `stale`: reports which entries reference modified source files
- `stale refresh=true`: shows stale entry + current source excerpt side by side
- `resolve_source`: path fallback — tries as-is, then one level of CWD subdirectories
- Staleness reduces effective confidence in the binary index

## Performance

Three tiers, each optimized for its latency budget:

1. **C FFI** (~200ns): zero-alloc binary index query, no IPC
   - `libamaranthine.dylib`, 9-function C API, pre-hashed terms
   - Hook path uses mmap(2) bypass — no socket round-trip
2. **MCP server** (~5ms): JSON-RPC over stdio, 26 tools, in-process dispatch
   - BufReader with reusable line buffer (no iterator allocation)
   - Stack-allocated IdBuf for JSON-RPC IDs (zero heap alloc for 99% of calls)
   - Arc<str> cached tool list (~15KB, built once)
3. **Corpus cache** (~0us warm, ~5ms cold): mtime-invalidated in-memory cache
   - All read paths use `cache::with_corpus` (zero disk I/O when warm)
   - Write paths still read from data.log directly

Key optimizations:
- **FxHash** (`fxhash.rs`): word-at-a-time multiply-rotate, ~3ns vs SipHash ~20ns
- **InternedStr** (`intern.rs`): Arc<str> for topic names, O(1) clone
- **ASCII fast-path tokenizer**: byte-level processing for 99%+ of content
- **tokenize_into_tfmap()**: builds FxHashMap during tokenization (eliminates ~57K String allocs on corpus load)
- **Lazy line extraction**: scores first (zero alloc), extracts lines only for top-K results
- **3-phase deferred snippets**: lightweight HeapHit, snippets only for final K hits
- **Pre-baked IDF**: computed at index build time, no log() at query time
- **Single-pass metadata extraction**: tags, source, confidence, links in one body scan
- **Zero format!() in hot paths**: push_str with pre-sized capacity throughout

## Data Flow

```
store(topic, text)
  -> build_body(): prepend metadata lines ([tags:], [source:], [confidence:], [links:])
  -> dupe check: 6-char word Jaccard at 90% threshold
  -> datalog::append_entry(): write to data.log
  -> inverted::ensure_index(): full rebuild of index.bin
  -> cache::invalidate(): clear corpus cache

search(query)
  -> cache::with_corpus(): load entries (warm: 0us, cold: ~5ms)
  -> score::search_scored(): BM25 with topic boost + tag boost + confidence
  -> search::run_*(): format output per detail level

brief(query)
  -> collect entries from matching topics + related via token overlap
  -> follow narrative links one level deep
  -> compress: Jaccard dedup + temporal chains + supersession
  -> briefing: classify into categories + format summary

index.bin layout:
  [Header 72B][TermTable][Postings][EntryMeta][Snippets][TopicTable][TopicNames][SourcePool][XrefTable]
  TermTable: open-addressing hash map (FNV-1a, power-of-two capacity)
  Postings: pre-baked IDF, per-entry TF
  EntryMeta: topic_id, word_count, snippet, date, source, confidence, log_offset

C FFI: amr_open -> amr_search_raw -> amr_snippet -> amr_close
Hook mmap: mmap(index.bin) -> binary search -> return snippets
```

## Decisions Made

- Single-file append-only log: never modify in place, delete = tombstone
- BM25 ranking: unified tokenizer with CamelCase/snake_case expansion
- Two separate stop word lists: search (conservative) vs store (broader for dedup)
- Zero dependencies: hand-rolled JSON, FxHash, InternedStr, binary index, date math
- Index v2: topics, xrefs, sources, log offsets in binary format
- `#[repr(C, packed)]` structs in `format.rs` for zero-copy access
- FNV-1a hash with zero-sentinel guard (hash 0 = empty slot)
- Hash table capacity always power-of-two (enables `& mask` indexing)
- IDF pre-baked into postings at index build time (no query-time log())
- Synchronous full index rebuild on every write (acceptable at human speed)
- exec() for reload (replaces process image, not spawn)
- Hooks use CLI binary (not daemon) — ~5ms cold start, no state between calls
- Ambient hook uses mmap(2) bypass — zero socket overhead, sub-millisecond
- FxHash over SipHash for internal data: no DoS concern, 7x faster hashing
- Arc<str> interning over String for topics: O(1) clone, transparent Deref to &str
- Byte-level ASCII tokenizer with Unicode fallback: 99%+ content is ASCII
- Confidence as min(explicit, staleness): entries degrade if source changes
- Narrative links follow one level only: prevents unbounded traversal
- Tool consolidation: one `search` with detail param, one `delete` with mode flags
- JSON parser uses f64 for numbers: supports confidence floats natively
- Briefing compression: 3-pass category classification, Jaccard similarity chains, freshness weighting

## Key Invariants

These are load-bearing. Violating them corrupts data or crashes.

- **data.log is append-only.** Never modify in place. All mutations append new records.
- **Entry header is exactly 12 bytes** (type:1 + topic_len:1 + body_len:4 + ts_min:4 + pad:2).
- **index.bin is rebuilt from scratch** after every write. Never incrementally updated.
- **Term hash 0 is the empty slot sentinel.** `hash_term()` returns 1 if FNV-1a computes 0.
- **All format.rs structs are `#[repr(C, packed)]`** — reads use `ptr::read_unaligned`.
- **Topic names max 255 bytes** (u8 length field).
- **Timestamps are i32 minutes-since-epoch.** Good until ~4085 CE.
- **JSON `Obj` is `Vec<(String, Value)>`**, not HashMap — preserves insertion order.

## Key Files

44 Rust files, ~9,800 lines. Organized by layer:

### Core Data Layer
| File | Lines | What |
|------|-------|------|
| `datalog.rs` | 212 | Append-only data log: read, write, compact, migrate |
| `format.rs` | 92 | Binary index on-disk structs, `#[repr(C, packed)]`, hash_term |
| `inverted.rs` | 418 | Index builder: data.log -> index.bin with BM25-ready postings |
| `binquery.rs` | 568 | Index reader: 3-phase deferred snippet search, ~200ns queries |
| `cache.rs` | 190 | Corpus cache: mtime-invalidated, pre-tokenized, interned topics |

### Search & Scoring
| File | Lines | What |
|------|-------|------|
| `score.rs` | 348 | BM25 engine: AND->OR fallback, topic/tag boost, confidence weighting |
| `search.rs` | 181 | Output formatting: full/medium/brief/count/topics/grouped |
| `text.rs` | 301 | Tokenizer: ASCII fast path, CamelCase/snake_case, tag parser |

### Write Path
| File | Lines | What |
|------|-------|------|
| `store.rs` | 285 | Entry creation: Jaccard dedup, auto-tags, confidence, links |
| `edit.rs` | 173 | Entry modification: update, append to entry, tag operations |
| `delete.rs` | 96 | Entry/topic deletion via tombstone records |

### Compression & Synthesis
| File | Lines | What |
|------|-------|------|
| `compress.rs` | 278 | Cross-topic dedup, Jaccard similarity chains, temporal chains |
| `briefing.rs` | 572 | Category classification, format_summary, body-keyword rescue |
| `reconstruct.rs` | 201 | One-shot synthesis: topic matching, link following, glob patterns |

### Codebase Analysis
| File | Lines | What |
|------|-------|------|
| `callgraph.rs` | 174 | Caller/callee tracing with configurable depth |
| `codepath.rs` | 180 | Access site search with context and categorization |
| `reverse.rs` | 409 | Architecture mapping: module relationships, exports, coupling |
| `crash.rs` | 232 | Stack frame parsing + crash pattern matching |
| `perf.rs` | 221 | Callgraph + allocation/lock/I/O antipattern detection |

### MCP Server
| File | Lines | What |
|------|-------|------|
| `mcp.rs` | 427 | JSON-RPC stdio loop, index management, Mach-O audit on reload |
| `mcp/tools.rs` | 230 | 26 tool schema definitions |
| `mcp/dispatch.rs` | 503 | Tool call routing, argument extraction, filter building |

### Browse & Stats
| File | Lines | What |
|------|-------|------|
| `topics.rs` | 135 | Topic listing, recent entries, preview formatting |
| `context.rs` | 97 | Session briefing: activity-weighted topics + velocity |
| `digest.rs` | 32 | One-bullet-per-entry summaries |
| `stats.rs` | 219 | Statistics, tag listing, index health |
| `export.rs` | 81 | JSON export/import with timestamp preservation |
| `xref.rs` | 94 | Cross-reference finder |
| `depgraph.rs` | 169 | Topic dependency graph with glob filtering |

### Infrastructure
| File | Lines | What |
|------|-------|------|
| `json.rs` | 406 | Recursive descent JSON parser, fast-path strings, escape_into |
| `fxhash.rs` | 82 | Word-at-a-time multiply-rotate hasher, ~3ns/op |
| `intern.rs` | 77 | InternedStr: Arc<str> newtype, O(1) clone for topic names |
| `time.rs` | 204 | Date math: minutes-since-epoch, relative dates, zero-format |
| `config.rs` | 193 | Directory resolution, path sanitization, source resolution |
| `lock.rs` | 31 | Unix flock() for write serialization |
| `compact.rs` | 113 | Duplicate detection within topics |
| `prune.rs` | 40 | Stale topic flagging |
| `migrate.rs` | 39 | Timestamp backfill for legacy entries |

### Entry Points
| File | Lines | What |
|------|-------|------|
| `main.rs` | 277 | CLI entry: arg parsing, subcommand dispatch, hook routing |
| `lib.rs` | 198 | Library root: module declarations + C FFI exports |
| `cffi.rs` | 124 | C FFI zero-alloc query path with generation counter |
| `hook.rs` | 499 | Hook handlers: mmap ambient, post-build, stop, subagent-start |
| `sock.rs` | 226 | Unix domain socket listener for hook queries |
| `install.rs` | 194 | Installer: binary copy, codesign, MCP config, hooks |
