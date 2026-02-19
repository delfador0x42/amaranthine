# Amaranthine — Persistent Knowledge Base

## What This Does
Fast, local CLI + library for storing and retrieving development knowledge
across AI-assisted coding sessions. Two access paths: MCP server (~5ms),
C FFI dylib (~200ns). Single append-only data log, binary inverted index,
zero dependencies, frictionless capture. No cloud, no database, no runtime deps.

## Why This Design
Each AI session starts with ~100 lines of context for a 50K-line project.
Topic files help but require knowing which file to read — circular when
you don't know what you don't know. amaranthine makes the right thing easy:
one command to store, one to search, one to reconstruct full understanding.

## Storage Architecture
Single-file append-only data log (`data.log`) + binary inverted index (`index.bin`):
- **Write path**: store → append EntryRecord to data.log → rebuild index.bin
- **Read path**: search/context/stats → BM25 over data.log entries
- **Fast path**: C FFI → query index.bin directly (~200ns, zero alloc)
- **Delete**: append DeleteRecord (tombstone) referencing target's byte offset
- **Edit**: append new EntryRecord + DeleteRecord for old version
- Topics are virtual metadata — no per-topic files, no filesystem overhead

### data.log format
Magic `b'AMRL'`. Two record types:
- Entry: `[0x01, topic_len:u8, body_len:u32, ts_min:i32, pad:2, topic, body]`
- Delete: `[0x02, pad:3, offset:u32]` — tombstones an entry by byte offset

### index.bin format
Magic `b'AMRN'` v2. Sections: Header → TermTable → Postings → EntryMeta →
Snippets → TopicTable → TopicNames → SourcePool → XrefTable.
All `#[repr(C, packed)]` structs for zero-copy mmap access.

## Search (v5.3)
Unified tokenizer (`text::tokenize`): byte-level ASCII fast path, split on
non-alphanumeric, expand CamelCase/snake_case, lowercase, min 2 chars.
Falls back to Unicode for non-ASCII content (<1% of entries).
- BM25 scoring with K1=1.2, B=0.75
- Topic-name boost: 1.5x multiplicative for entries in matching topics
- Tag-aware scoring: +30% per query term matching entry tags
- AND→OR fallback: multi-word queries retry as OR when AND returns 0 results
- Conservative SEARCH_STOP_WORDS (pure function words, no technical terms)
- FxHash (`fxhash.rs`) for all internal HashMap/HashSet (~3ns vs SipHash ~20ns)

## Hooks (v5.1)
Four Claude Code hooks in `src/hook.rs`, dispatched via `amaranthine hook <type>`:
- **ambient** (PreToolUse): queries binary index on file stem before Read/Edit/Write
- **post-build** (PostToolUse Bash): reminds to store build findings
- **stop** (Stop): debounced 120s reminder to persist findings
- **subagent-start** (SubagentStart): dynamic topic list from index
Installed globally to `~/.claude/settings.json` by `amaranthine install`.

## Staleness Detection
Entries with `[source: path/to/file:line]` metadata track source file provenance.
- `check_stale`: reports which entries reference modified source files
- `refresh_stale`: shows stale entry + current source excerpt side by side
- `resolve_source`: path fallback — tries as-is, then one level of CWD subdirectories

## Performance (v5.3)
Three tiers, each optimized for its latency budget:

1. **C FFI** (~200ns): zero-alloc binary index query, no IPC
   - `libamaranthine.dylib`, 9-function C API, pre-hashed terms
2. **MCP server** (~5ms): JSON-RPC over stdio, 37 tools, in-process dispatch
3. **Corpus cache** (~0µs warm, ~5ms cold): mtime-invalidated in-memory cache
   - All read-only paths use `cache::with_corpus` (zero disk I/O when warm)
   - Write paths (`store`, `delete`, `edit`) still read from data.log directly

Cache reload optimizations (v5.3):
- **InternedStr** (`intern.rs`): Arc<str> newtype for topic names. ~45 unique
  topics shared across ~1000 entries. Clone = atomic refcount bump, no heap alloc.
  Deref<Target=str> + PartialEq<str/&str/String> for transparent caller compat.
- **FxHash** (`fxhash.rs`): non-cryptographic hasher (~3ns vs SipHash ~20ns).
  Multiply-rotate with seed. Used for token_set, tf_map, score counters.
- **Single-pass construction**: build tf_map from tokens, derive token_set from
  tf_map.keys(). Eliminates redundant clone+hash pass.
- **ASCII fast-path tokenizer**: byte-level processing for ASCII content (99%+),
  skip UTF-8 decode. CamelCase detected via `is_ascii_uppercase()` on raw bytes.
- **Pre-sized allocations**: `with_capacity` on all hot-path Vecs.
- **Zero redundant lowercasing**: topics/tags stored lowercase by convention
  (`config::sanitize_topic`, `store::normalize_tags`). No runtime to_lowercase.
- `target-cpu=native`, `#[inline]` on cross-module hot functions.

## Data Flow
`store` → append to `data.log` → rebuild `index.bin` → reload in-memory
`search` → BM25 on data.log entries, AND→OR fallback, topic/tag/date filtering
`index_search` → in-memory binary index query (~200ns)
`reconstruct` → collect matching entries → compress → hierarchical output
`serve` → MCP server over stdio (JSON-RPC, 37 tools)
C FFI → `amr_open` → `amr_search_raw` → `amr_snippet` → `amr_close`

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
- FxHash over SipHash for internal data: no DoS concern, 7x faster hashing
- Arc<str> interning over String for topics: O(1) clone, transparent Deref to &str
- Byte-level ASCII tokenizer with Unicode fallback: 99%+ content is ASCII

## Key Files
- `src/lib.rs` — library root: pub modules + C FFI (amr_open/search/close)
- `src/main.rs` — CLI entry, manual arg parsing, hook dispatch
- `src/hook.rs` — Claude Code hook handlers (ambient, post-build, stop, subagent-start)
- `src/datalog.rs` — append-only data log: read, write, migrate, compact
- `src/format.rs` — binary index v2 on-disk structs + hash_term + as_bytes
- `src/inverted.rs` — index v2 builder (reads data.log, produces index.bin)
- `src/binquery.rs` — index v2 query engine (BM25 search + metadata readers)
- `src/cffi.rs` — C FFI zero-alloc query path (~200ns search_raw + snippets)
- `src/text.rs` — ASCII-fast tokenizer, query terms, CamelCase splitting
- `src/fxhash.rs` — FxHash: non-cryptographic hasher for internal data (~3ns/op)
- `src/intern.rs` — InternedStr: Arc<str> newtype with rich PartialEq impls
- `src/cache.rs` — corpus cache: mtime-invalidated, pre-tokenized, interned topics
- `src/search.rs` — BM25 search + topic/tag boost + multiple output formats
- `src/store.rs` — entry creation with 6-char word Jaccard dedup
- `src/compress.rs` — v5 compression engine: cross-topic dedup + temporal chains
- `src/reconstruct.rs` — semantic synthesis: tag-categorized hierarchical output
- `src/config.rs` — directory resolution, source path resolution, staleness checks
- `src/stats.rs` — statistics, staleness checking, refresh_stale
- `src/install.rs` — installer: binary, MCP config, CLAUDE.md, hooks
- `src/mcp.rs` — MCP server loop + state + startup
- `src/mcp/tools.rs` — 37 tool schema definitions
- `src/mcp/dispatch.rs` — tool call routing + arg helpers
- `src/json.rs` — recursive descent JSON parser + pretty printer
