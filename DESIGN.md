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

## Search (v5.1)
Unified tokenizer (`text::tokenize`): split on non-alphanumeric, expand
CamelCase/snake_case, lowercase, min 2 chars. Used by search, index builder,
and query terms — consistent tokenization across all paths.
- BM25 scoring with K1=1.2, B=0.75
- Topic-name boost: 1.5x multiplicative for entries in matching topics
- Tag-aware scoring: +30% per query term matching entry tags
- AND→OR fallback: multi-word queries retry as OR when AND returns 0 results
- Conservative SEARCH_STOP_WORDS (pure function words, no technical terms)

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

## Performance
1. **MCP server** (~5ms): JSON-RPC over stdio, 37 tools, in-process dispatch
2. **C FFI dylib** (~200ns): direct in-process query, no IPC
   - `libamaranthine.dylib` with 9-function C API
   - Binary inverted index v2: BM25 scoring, FNV-1a hashing, open addressing
   - Zero-alloc search path with pre-hashed terms

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
- Zero dependencies: hand-rolled JSON, libc FFI, binary index, date math
- Index v2: topics, xrefs, sources, log offsets in binary format
- `#[repr(C, packed)]` structs in `format.rs` for zero-copy access
- FNV-1a hash with zero-sentinel guard (hash 0 = empty slot)
- Hash table capacity always power-of-two (enables `& mask` indexing)
- IDF pre-baked into postings at index build time (no query-time log())
- Synchronous full index rebuild on every write (acceptable at human speed)
- exec() for reload (replaces process image, not spawn)
- Hooks use CLI binary (not daemon) — ~5ms cold start, no state between calls

## Key Files
- `src/lib.rs` — library root: pub modules + C FFI (amr_open/search/close)
- `src/main.rs` — CLI entry, manual arg parsing, hook dispatch
- `src/hook.rs` — Claude Code hook handlers (ambient, post-build, stop, subagent-start)
- `src/datalog.rs` — append-only data log: read, write, migrate, compact
- `src/format.rs` — binary index v2 on-disk structs + hash_term + as_bytes
- `src/inverted.rs` — index v2 builder (reads data.log, produces index.bin)
- `src/binquery.rs` — index v2 query engine (BM25 search + metadata readers)
- `src/cffi.rs` — C FFI zero-alloc query path (~200ns search_raw + snippets)
- `src/text.rs` — unified tokenizer, query terms, CamelCase splitting
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
