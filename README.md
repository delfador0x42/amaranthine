# amaranthine

Persistent memory for AI coding agents. Your agent forgets everything between sessions — amaranthine fixes that.

Single-file append-only data store, binary inverted index, BM25 search, 26 MCP tools, zero dependencies.

> **Platform:** macOS (Apple Silicon and Intel). Linux support is straightforward but untested — codesign steps are skipped automatically on non-macOS.

## Install

**Requirements:** [Rust toolchain](https://rustup.rs/) (`cargo`) and [Claude Code](https://docs.anthropic.com/en/docs/claude-code).

```bash
git clone https://github.com/talsec/amaranthine.git
cd amaranthine
make install
```

That's it. `make install` builds the binary and runs the installer, which:
1. Builds the release binary
2. Copies it to `~/.local/bin/amaranthine` and codesigns it (macOS)
3. Creates `~/.amaranthine/` for knowledge storage
4. Registers the MCP server in `~/.claude.json`
5. Adds agent instructions to `~/.claude/CLAUDE.md`
6. Installs 4 hooks in `~/.claude/settings.json`

**Restart Claude Code** after install. Your agent now has persistent memory.

> **Note:** The MCP server uses the full binary path, so `~/.local/bin` doesn't need to
> be on your PATH. Add it if you want CLI access: `export PATH="$HOME/.local/bin:$PATH"`

### Verify

Quick check (no Claude Code needed):

```bash
~/.local/bin/amaranthine store test "install verification" --tags test
~/.local/bin/amaranthine search "install verification"
~/.local/bin/amaranthine call delete topic="test" all="true"
```

You should see your entry in the search results. Then open Claude Code and ask your agent to search amaranthine for something — it should use the `search` MCP tool automatically.

### Troubleshooting

**`cargo: command not found`** — Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

**MCP server not showing up** — Check `~/.claude.json` has `amaranthine` under `mcpServers`. Re-run `make install` if missing. Restart Claude Code.

**`codesign` errors (macOS)** — The binary needs ad-hoc signing: `codesign -s - -f ~/.local/bin/amaranthine`

**Hooks not firing** — Check `~/.claude/settings.json` has entries under `hooks.PreToolUse`, `hooks.PostToolUse`, `hooks.Stop`, and `hooks.SubagentStart`. Re-run `make install` if missing.

**Permission denied** — `chmod +x ~/.local/bin/amaranthine`

### Uninstall

```bash
rm ~/.local/bin/amaranthine
rm -rf ~/.amaranthine/                    # deletes all stored knowledge
```

Then remove `amaranthine` from `~/.claude.json` (under `mcpServers`), hook entries from `~/.claude/settings.json`, and the `## Memory — amaranthine` section from `~/.claude/CLAUDE.md`.

## What happens

Every session, the agent can:

- **Store** findings as it works — tagged, timestamped, source-linked
- **Search** across everything it's ever stored — BM25 ranked, AND-to-OR fallback
- **Brief** — one-shot compressed briefings that reconstruct full understanding of a topic
- **Track staleness** — entries linked to source files know when those files change

Four hooks run automatically:

| Hook | When | What |
|------|------|------|
| **ambient** | Before file reads/edits | Injects relevant knowledge from the index |
| **post-build** | After build commands | Reminds to store findings |
| **stop** | Session ending | Reminds to persist discoveries |
| **subagent** | Subagent starting | Injects topic list for context |

## Tools

26 MCP tools, grouped by function:

**Core** — `store`, `batch`, `search` (BM25 with detail levels: full/medium/brief/count/topics/grouped), `brief` (one-shot compressed briefings with glob patterns, temporal filters)

**Write** — `append`, `delete`, `revise`, `tag`, `rename`, `merge`

**Browse** — `read`, `topics`, `recent`, `entries`, `stats`

**Analysis** — `trace` (callgraph, codepath, reverse-map, core/dead code, simplify, crash, perf), `stale`, `xref`, `graph`

**Maintenance** — `compact`, `prune`, `export`, `import`, `reindex`, `session`, `_reload`

## CLI

amaranthine also works from the command line:

```bash
amaranthine store rust-tips "always use #[repr(C)] for FFI structs" --tags rust,ffi
amaranthine search "FFI"
amaranthine search "FFI" --brief
amaranthine context --brief
amaranthine recent 3
amaranthine topics
```

## How it works

All knowledge lives in a single append-only file:

```
~/.amaranthine/
  data.log       # entries + tombstone deletes
  index.bin      # binary inverted index, rebuilt on write
```

Topics are metadata on entries, not separate files. Entries carry timestamps and optional metadata: tags (`[tags: rust, ffi]`), source links (`[source: src/main.rs:42]`), confidence (`[confidence: 0.8]`), and narrative links (`[links: topic:idx]`).

Search uses BM25 with CamelCase/snake_case splitting, topic-name boost, tag-aware scoring, and AND-to-OR fallback. A binary inverted index enables ~200ns queries for the hook path (mmap bypass, zero socket overhead).

## Architecture

- Zero runtime dependencies — hand-rolled JSON parser, hasher, binary index, date math
- 44 Rust files, ~9,800 lines, ~1.3MB release binary
- Three access tiers: C FFI (~200ns), MCP server (~5ms), corpus cache (~0us warm)
- See [DESIGN.md](DESIGN.md) for architecture details
- See [CONTRIBUTING.md](CONTRIBUTING.md) for development guide
- See [prompts/deep-refactor.md](prompts/deep-refactor.md) for the deep knowledge seeding prompt
