# Amaranthine — Persistent Knowledge Base

## What This Does
Fast, local CLI for storing and retrieving development knowledge across
AI-assisted coding sessions. Plain markdown files, ripgrep-fast search,
frictionless capture. No cloud, no database, no runtime dependencies.

## Why This Design
The problem: each AI session starts with ~100 lines of context for a 50K-line
project. Topic files help but require knowing which file to read — circular
when you don't know what you don't know. mem0 adds semantic search but has
API latency and noisy results. amaranthine makes the right thing easy:
one command to store, one command to search, section-based results.

## Data Flow
`store` → append timestamped entry to `<topic>.md` in memory dir
`search` → regex/substring across all `.md` files, return matching sections
`index` → scan topic files, generate `INDEX.md` manifest
`recent` → filter entries by date header within last N days
`prune` → flag topic files with no updates in N days

## Decisions Made
- Plain markdown (not SQLite): human-readable, git-trackable, grep-able
- Section-based search (not line-based): shows full context of each match
- Case-insensitive by default: matches how you think, not how you typed
- No vector embeddings: substring + regex covers 95% of recall for <1MB of files
- Timestamps in `## YYYY-MM-DD HH:MM` format: parseable, sortable, readable
- Excludes INDEX.md from search/topics, includes MEMORY.md in search only

## Key Files
- `src/main.rs` — CLI entry, clap dispatch
- `src/config.rs` — path resolution, file listing helpers
- `src/store.rs` — timestamped entry append
- `src/search.rs` — section-based regex search
- `src/index.rs` — manifest generation
- `src/topics.rs` — list + recent entries
- `src/prune.rs` — staleness detection
