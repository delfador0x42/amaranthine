# amaranthine

Persistent memory for AI coding agents. Your agent forgets everything between sessions — amaranthine fixes that.

Single-file append-only data store, binary inverted index, BM25 search, 37 MCP tools, zero dependencies.

> **Platform:** macOS (Apple Silicon and Intel). Linux support is straightforward but untested — codesign steps are skipped automatically on non-macOS.

## Install

**Requirements:** [Rust toolchain](https://rustup.rs/) (`cargo`) and [Claude Code](https://docs.anthropic.com/en/docs/claude-code).

```bash
git clone https://github.com/talsec/amaranthine.git
cd amaranthine
make install
```

That's it. `make install` handles everything:
1. Builds the release binary (~15s)
2. Copies it to `~/.local/bin/amaranthine` and codesigns it (macOS)
3. Creates `~/.amaranthine/` for knowledge storage
4. Registers the MCP server in `~/.claude.json`
5. Adds agent instructions to `~/.claude/CLAUDE.md`
6. Installs 4 hooks in `~/.claude/settings.json`

**Restart Claude Code** (or start a new session). Your agent now has persistent memory.

> **Note:** The MCP server uses the full binary path, so `~/.local/bin` doesn't need to
> be on your PATH. Add it if you want CLI access: `export PATH="$HOME/.local/bin:$PATH"`

### Verify it works

Quick CLI check (no Claude Code needed):

```bash
~/.local/bin/amaranthine store test "install verification" --tags test
~/.local/bin/amaranthine search "install verification"
~/.local/bin/amaranthine call delete topic="test" all="true"
```

You should see your entry in the search results. If you do, everything is wired up.

Then open Claude Code and try it end-to-end — ask your agent to search amaranthine for something. The agent should use the `search` MCP tool automatically.

### Troubleshooting

**`cargo: command not found`** — Install the Rust toolchain: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

**MCP server not showing up in Claude Code** — Check `~/.claude.json` has an `amaranthine` entry under `mcpServers`. If not, re-run `make install`. Make sure you restarted Claude Code after install.

**`codesign` errors** — Only affects macOS. The binary needs ad-hoc signing so macOS doesn't kill it. Run: `codesign -s - -f ~/.local/bin/amaranthine`

**Hooks not firing** — Check `~/.claude/settings.json` has entries under `hooks.PreToolUse`, `hooks.PostToolUse`, `hooks.Stop`, and `hooks.SubagentStart`. If missing, re-run `make install`.

**Permission denied** — Make sure `~/.local/bin/amaranthine` is executable: `chmod +x ~/.local/bin/amaranthine`

### Uninstall

```bash
rm ~/.local/bin/amaranthine                          # binary
rm -rf ~/.amaranthine/                               # knowledge data (⚠️ deletes all stored knowledge)
```

Then remove the `amaranthine` entry from `~/.claude.json` (under `mcpServers`) and the hook entries from `~/.claude/settings.json`. The installer appends a section to `~/.claude/CLAUDE.md` — remove the `## Memory — amaranthine` section if present.

## What happens

Every session, the agent can:

- **Store** findings as it works — tagged, timestamped, source-linked
- **Search** across everything it's ever stored — BM25 ranked, AND-to-OR fallback
- **Reconstruct** full understanding of a topic from stored knowledge (one-shot briefings)
- **Track staleness** — entries linked to source files know when those files change

Four hooks run automatically in the background:

| Hook | When | What |
|------|------|------|
| **ambient** | Before file reads/edits | Injects relevant knowledge from the index |
| **post-build** | After build commands | Reminds to store findings |
| **stop** | Session ending | Reminds to persist discoveries |
| **subagent** | Subagent starting | Injects topic list for context |

## Tools

The agent gets 37 MCP tools, grouped by function:

**Search** — `search` (BM25, with detail levels: full/medium/brief/count/topics), `search_entity` (grouped by topic), `index_search` (binary index, ~200ns)

**Write** — `store` (with optional tags, source links, confidence, narrative links), `batch_store`, `append`, `append_entry`, `update_entry`, `delete`

**Browse** — `context`, `topics`, `recent`, `read_topic`, `digest`, `stats`, `list_tags`, `list_entries`, `get_entry`

**Analysis** — `reconstruct` (compressed briefing with link-following), `search_entity`, `dep_graph`, `xref`, `check_stale`, `refresh_stale`, `codepath`

**Edit** — `rename_topic`, `merge_topics`, `tag_entry`

**Maintenance** — `compact`, `prune`, `migrate`, `export`, `import`, `session`, `rebuild_index`, `index_stats`, `compact_log`, `_reload`

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

Topics are metadata on entries — no per-topic files. Entries are timestamped and can carry tags (`[tags: rust, ffi]`), source links (`[source: src/main.rs:42]`), confidence levels (`[confidence: 0.8]`), and narrative links (`[links: topic:idx]`) to other entries.

Search uses BM25 with CamelCase/snake_case splitting, topic-name boost, tag-aware scoring, and AND-to-OR fallback. A binary inverted index enables ~200ns queries for the C FFI path (used by hooks).

## Architecture

- Zero runtime dependencies — hand-rolled JSON parser, hasher, binary index, date math
- 41 Rust files, ~7700 lines, ~1.1MB release binary
- Three access tiers: C FFI (~200ns), MCP server (~5ms), corpus cache (~0us warm)
- See [DESIGN.md](DESIGN.md) for architecture details
- See [CONTRIBUTING.md](CONTRIBUTING.md) for development guide
- See [prompts/deep-refactor.md](prompts/deep-refactor.md) for the deep knowledge seeding prompt
