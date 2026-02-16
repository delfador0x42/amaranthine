# Amaranthine — Persistent Knowledge Base

## What This Does
Fast, local CLI for storing and retrieving development knowledge across
AI-assisted coding sessions. Plain markdown files, zero dependencies,
frictionless capture. No cloud, no database, no runtime deps.

## Why This Design
Each AI session starts with ~100 lines of context for a 50K-line project.
Topic files help but require knowing which file to read — circular when
you don't know what you don't know. amaranthine makes the right thing easy:
one command to store, one to search, one to orient at session start.

## Data Flow
`store` → append timestamped entry to `<topic>.md` (warns on duplicates, stdin via -)
`batch_store` → native JSON array of {topic, text, tags?}, terse default, intra-batch dedup
`append` → add text to last entry (no new timestamp, for related follow-ups)
`append_entry` → add to specific entry by match/index/tag
`search` → BM25 ranked search, topic filter, detail level (full/medium/brief), AND→OR fallback
`context` → combined topics + recent + optional search (--brief: topics only)
`delete` → remove last entry (--last), by match (--match), or entire topic (--all)
`edit` → replace matching entry content in-place (keeps timestamp)
`recent` → filter entries by date header within last N days
`session` → show what was stored this session (static Mutex log)
`serve` → MCP server over stdio (JSON-RPC, in-process dispatch, 31 tools)
`install` → self-install to ~/.claude.json + CLAUDE.md

## Decisions Made
- Plain markdown: human-readable, git-trackable, grep-able
- BM25 ranking: relevance-ordered results, CamelCase/snake_case aware
- AND→OR fallback: strict matching first, broaden if no results
- Zero dependencies: hand-rolled JSON parser, libc FFI, substring search
- 640KB binary, <5s compile: every line is ours
- Timestamps in `## YYYY-MM-DD HH:MM` format: parseable, sortable
- MCP server dispatches in-process: all modules return Result<String, String>
- Hand-rolled JSON parser: recursive descent, ~200 lines, handles full spec
- `batch_store` native array schema: MCP sends entries as JSON array, not string
- `search` query optional: browse by topic/tag without a search query
- `search` detail param: full/medium/brief controls output verbosity, medium default
- `session` tool: static Mutex<Vec<String>> tracks stores without external state
- `append_entry` tag param: find most recent entry with tag, append to it
- Soft dupe detection: warns in Ok() instead of blocking with Err()
- Intra-batch dedup: tracks (topic, text_prefix) tuples within a single batch_store
- Atomic writes: tmp + fsync + rename prevents corruption on crash

## Key Files
- `src/main.rs` — CLI entry, manual arg parsing
- `src/json.rs` — recursive descent JSON parser + pretty printer
- `src/mcp.rs` — MCP server: stdio loop, 31 tools, session tracking, in-process dispatch
- `src/install.rs` — self-install to ~/.claude.json + CLAUDE.md
- `src/time.rs` — libc FFI (localtime_r), Hinnant date algorithm
- `src/config.rs` — path resolution, sanitization, atomic_write, file listing
- `src/context.rs` — session orientation (topics + recent + search)
- `src/store.rs` — timestamped entry append + dupe detection + tag singularization
- `src/search.rs` — BM25 search + topic filter + tag extraction + multiple output formats
- `src/delete.rs` — entry/topic removal + split_sections parser
- `src/edit.rs` — in-place replacement + match/index/tag append + rename_topic + tag_entry
- `src/digest.rs` — compact summary generator
- `src/topics.rs` — list (with preview) + recent entries
- `src/prune.rs` — staleness detection
- `src/lock.rs` — file locking for concurrent access
