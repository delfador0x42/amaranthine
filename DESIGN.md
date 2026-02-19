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

## Performance
1. **MCP server** (~5ms): JSON-RPC over stdio, 36 tools, in-process dispatch
2. **C FFI dylib** (~200ns): direct in-process query, no IPC
   - `libamaranthine.dylib` with 9-function C API
   - Binary inverted index v2: BM25 scoring, FNV-1a hashing, open addressing
   - Zero-alloc search path with pre-hashed terms

## Data Flow
`store` → append to `data.log` → rebuild `index.bin` → reload in-memory
`search` → BM25 on data.log entries, AND→OR fallback, topic/tag/date filtering
`index_search` → in-memory binary index query (~200ns)
`reconstruct` → collect matching entries → group by semantic tag category →
  size-budgeted hierarchical output (ARCHITECTURE, INVARIANTS, GOTCHAS, etc.)
`serve` → MCP server over stdio (JSON-RPC, 36 tools)
C FFI → `amr_open` → `amr_search_raw` → `amr_snippet` → `amr_close`

## Decisions Made
- Single-file append-only log: never modify in place, delete = tombstone
- BM25 ranking: CamelCase/snake_case aware via shared `text::query_terms`
- Zero dependencies: hand-rolled JSON, libc FFI, binary index, date math
- Index v2: topics, xrefs, sources, log offsets in binary format
- `#[repr(C, packed)]` structs in `format.rs` for zero-copy access
- FNV-1a 64-bit hash: fast, good distribution, no deps
- lib.rs + main.rs split: library crate (modules + C FFI) + binary consumer
- mcp.rs split: server loop + tools schema + dispatch routing (3 files)
- Shared format contract: `format.rs` used by both builder and reader
- FFI query path in `cffi.rs`: separated from MCP query path
- Semantic tag categorization in reconstruct: tags drive grouping, not topics

## Key Files
- `src/lib.rs` — library root: pub modules + C FFI (amr_open/search/close)
- `src/main.rs` — CLI entry, manual arg parsing
- `src/datalog.rs` — append-only data log: read, write, migrate, compact
- `src/format.rs` — binary index v2 on-disk structs + hash_term + as_bytes
- `src/inverted.rs` — index v2 builder (reads data.log, produces index.bin)
- `src/binquery.rs` — index v2 query engine (BM25 search + metadata readers)
- `src/cffi.rs` — C FFI zero-alloc query path (~200ns search_raw + snippets)
- `src/text.rs` — shared text utilities (query_terms, CamelCase splitting)
- `src/search.rs` — BM25 search + filtering + multiple output formats
- `src/reconstruct.rs` — semantic synthesis: tag-categorized hierarchical output
- `src/mcp.rs` — MCP server loop + state + startup
- `src/mcp/tools.rs` — 36 tool schema definitions
- `src/mcp/dispatch.rs` — tool call routing + arg helpers
- `src/json.rs` — recursive descent JSON parser + pretty printer
