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
`append` → add text to last entry (no new timestamp, for related follow-ups)
`search` → case-insensitive substring, return matching sections (--brief: topic + first hit)
`context` → combined topics + recent + optional search (--brief: topics only)
`delete` → remove last entry (--last), by match (--match), or entire topic (--all)
`edit` → replace matching entry content in-place (keeps timestamp)
`index` → scan topic files, generate `INDEX.md` manifest
`recent` → filter entries by date header within last N days
`prune` → flag topic files with no updates in N days
`serve` → MCP server over stdio (JSON-RPC, in-process dispatch)
`install` → self-install to ~/.claude.json + CLAUDE.md

## Decisions Made
- Plain markdown: human-readable, git-trackable, grep-able
- Section-based search: shows full context of each match
- Case-insensitive by default: matches how you think, not how you typed
- Zero dependencies: hand-rolled arg parsing, libc FFI, substring search
- `--plain` flag: strips ANSI for programmatic use (AI tool calls)
- 470KB binary, 3s compile: every line is ours
- Timestamps in `## YYYY-MM-DD HH:MM` format: parseable, sortable
- MCP server dispatches in-process: all modules return Result<String, String>
  (v0.3 used subprocesses — broke in sandboxed environments like Claude Code)
- Hand-rolled JSON parser: recursive descent, ~200 lines, handles full spec
- `install` modifies ~/.claude.json directly via own JSON parser (dogfooding)
- Single knowledge dir: ~/.amaranthine/ always, --dir for explicit override only
- `append` adds to last entry (no timestamp) — prevents topic clutter for related info
- Dupe detection on `store`: 60%+ word overlap triggers warning (still stores)
- `context --brief`: topics only, no recent — fast orientation for quick tasks

## Key Files
- `src/main.rs` — CLI entry, manual arg parsing
- `src/json.rs` — recursive descent JSON parser + pretty printer
- `src/mcp.rs` — MCP server: stdio loop, 14 tools, in-process dispatch
- `src/install.rs` — self-install to ~/.claude.json + CLAUDE.md
- `src/time.rs` — libc FFI (localtime_r), Hinnant date algorithm
- `src/config.rs` — path resolution, sanitization, file listing
- `src/context.rs` — session orientation (topics + recent + search)
- `src/store.rs` — timestamped entry append + append-to-last + dupe detection
- `src/search.rs` — section-based substring search
- `src/delete.rs` — entry/topic removal + split_sections parser
- `src/edit.rs` — in-place entry replacement + match-based append
- `src/digest.rs` — compact summary generator
- `src/index.rs` — manifest generation
- `src/topics.rs` — list + recent entries
- `src/prune.rs` — staleness detection
