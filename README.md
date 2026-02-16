# amaranthine

Persistent knowledge base for AI coding agents. Zero dependencies, 443KB binary.

Your coding agent forgets everything between sessions. amaranthine gives it a
memory that persists — plain markdown files it can store, search, and read.

## Install

```bash
cd amaranthine && cargo build --release && sudo cp target/release/amaranthine /usr/local/bin/ && amaranthine install
```

That's it. `amaranthine install` does three things:
1. Creates `~/.amaranthine/` (where all knowledge lives)
2. Adds MCP server to `~/.claude/settings.json`
3. Adds usage instructions to `~/.claude/CLAUDE.md`

Restart Claude Code. Your agent now has 9 tools for persistent memory.

## What the agent gets

After install, these MCP tools appear automatically:

| Tool | What it does |
|------|-------------|
| `store` | Save knowledge under a topic |
| `search` | Find knowledge across all topics |
| `context` | Session briefing (topics + recent) |
| `read_topic` | Read a full topic file |
| `digest` | Compact summary of everything |
| `topics` | List all topics with counts |
| `recent` | Entries from last N days |
| `delete_entry` | Remove last entry from a topic |
| `delete_topic` | Delete an entire topic |

## How it works

Knowledge is stored as timestamped markdown entries in topic files:

```
~/.amaranthine/
  rust-ffi.md        # 7 entries about Rust FFI patterns
  iris-project.md    # project structure, build commands
  iris-gotchas.md    # things that bit me once
```

Each file is human-readable markdown. No database, no cloud, no lock-in.

The MCP server (`amaranthine serve`) speaks JSON-RPC over stdio. Claude Code
starts it automatically and talks to it through stdin/stdout. Each tool call
runs the CLI as a subprocess — zero code duplication between CLI and server.

## CLI usage

```bash
amaranthine store rust-tips "always use #[repr(C)] for FFI structs"
amaranthine search "FFI"
amaranthine context                    # session briefing
amaranthine digest                     # compact summary for MEMORY.md
amaranthine recent 3                   # last 3 days
amaranthine topics                     # list all topics
amaranthine delete rust-tips --last    # remove last entry
echo "multi-line\nentry" | amaranthine store notes -   # stdin support
```

`--plain` strips ANSI colors (used by MCP server for clean tool output).

## Design

- Zero runtime dependencies — hand-rolled arg parsing, libc FFI, JSON parser
- 443KB stripped binary, 3s release compile
- All knowledge in `~/.amaranthine/`, override with `--dir` or `AMARANTHINE_DIR`
- See [DESIGN.md](DESIGN.md) for architecture decisions
