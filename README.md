# amaranthine

Persistent knowledge base for AI coding agents. Zero dependencies, ~640KB binary.

Your coding agent forgets everything between sessions. amaranthine gives it a
memory that persists — plain markdown files it can store, search, and read.

## Install

```bash
git clone <repo> && cd amaranthine && make install
```

That's it. `make install` builds the binary, then runs `amaranthine install` which:
1. Copies the binary to `~/.local/bin/amaranthine`
2. Codesigns it (macOS — required or taskgate kills it)
3. Creates `~/.amaranthine/` (where all knowledge lives)
4. Adds MCP server config to `~/.claude.json`
5. Adds usage instructions to `~/.claude/CLAUDE.md`

Restart Claude Code. Your agent now has persistent memory.

**Requirements:** Rust toolchain (`cargo`). No other dependencies.

## What the agent gets

31 MCP tools, organized by function:

**Search** — find knowledge across all topics
| Tool | What it does |
|------|-------------|
| `search` | Full-text BM25 search, topic filter, detail level (full/medium/brief) |
| `search_medium` | Topic + timestamp + first 2 lines per hit (default) |
| `search_brief` | Topic + first matching line per hit |
| `search_topics` | Which topics matched + hit counts |
| `search_count` | Just count matches (fastest) |

**Write** — store and modify knowledge
| Tool | What it does |
|------|-------------|
| `store` | Save a timestamped entry under a topic |
| `batch_store` | Store multiple entries as native JSON array (terse default, intra-batch dedup) |
| `append` | Add text to the last entry (no new timestamp) |
| `append_entry` | Add text to a specific entry by match, index, or tag |
| `update_entry` | Replace an entry's text (keeps timestamp) |
| `delete_entry` | Remove an entry by match/index/last |
| `delete_topic` | Delete an entire topic |

**Browse** — explore what's stored
| Tool | What it does |
|------|-------------|
| `context` | Session briefing: topics + recent entries |
| `topics` | List all topics with entry counts |
| `recent` | Entries from last N days/hours |
| `read_topic` | Read a full topic file |
| `digest` | One-bullet summary of every entry |
| `stats` | Topic count, entry count, date range |
| `list_tags` | All tags with usage counts |
| `list_entries` | Entries in a topic with index numbers |
| `get_entry` | Fetch a single entry by topic + index |

**Edit** — reorganize knowledge
| Tool | What it does |
|------|-------------|
| `rename_topic` | Rename a topic file (preserves entries) |
| `tag_entry` | Add or remove tags on an existing entry |

**Maintenance** — keep knowledge clean
| Tool | What it does |
|------|-------------|
| `compact` | Find and merge duplicate entries |
| `prune` | Flag stale topics (no entries in N days) |
| `xref` | Find cross-references between topics |
| `export` / `import` | JSON backup and restore |
| `migrate` | Fix entries without timestamps |
| `session` | Show what was stored this session |
| `_reload` | Hot-reload binary after code changes |

## How it works

Knowledge is stored as timestamped markdown entries in topic files:

```
~/.amaranthine/
  rust-ffi.md        # 12 entries about Rust FFI patterns
  build-gotchas.md   # things that bit me once
  iris-engine.md     # detection pipeline architecture
```

Each file is human-readable markdown. No database, no cloud, no lock-in.

Search uses BM25 ranking with CamelCase/snake_case splitting and AND-to-OR
fallback. Query is optional — browse by topic or tag without specifying one.
Duplicate detection warns but doesn't block. Tags are auto-normalized.

## CLI usage

```bash
amaranthine store rust-tips "always use #[repr(C)] for FFI structs" --tags rust,ffi
amaranthine search "FFI"
amaranthine search "FFI" --brief           # quick results
amaranthine search "FFI" --topics          # which topics matched
amaranthine context --brief                # session briefing
amaranthine recent 3                       # last 3 days
amaranthine topics                         # list all topics
amaranthine delete rust-tips --last        # remove last entry
```

## Design

- Zero runtime dependencies — hand-rolled JSON parser, arg parsing, libc FFI
- ~640KB stripped binary, <5s release compile
- All knowledge in `~/.amaranthine/`, override with `--dir` or `AMARANTHINE_DIR`
- MCP server speaks JSON-RPC over stdio (`amaranthine serve`)
- See [DESIGN.md](DESIGN.md) for architecture decisions
