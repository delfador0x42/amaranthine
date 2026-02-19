# Deep Codebase Refactor and Knowledge Seed 


## Phase 0: Load Existing Knowledge (unchanged, slightly tighter)
## Phase 1: Ground Truth Verification (unchanged)
## Phase 2: Module Map (enhanced template with Types + Called by/Uses)
## Phase 3: Structural Coupling (NEW — codepath + coupling profiles)
## Phase 4: Cross-Cutting Analysis (enhanced with transformation rules + playbooks)
## Phase 5: Deep Analysis (unchanged)
## Phase 6: Gap Seeding (enhanced with structural gaps)

## Core Principle

Understanding = compression. Code is the artifact of understanding, not the goal.

Your mission: pre-compress this codebase into amaranthine's LLM-native format —
hierarchical, tagged, source-linked, freshness-annotated, gap-seeded — so that
any future session can instantly decompress a complete mental model via
`reconstruct`. No code changes. Zero. The only output is knowledge.

When the compression is lossless, you feel like you've lived in the codebase for
six months. You know the architecture, the gotchas, the dead code, the phantom
wiring, the coupling — and you got it all from one `reconstruct` call.

## The One-Shot Test

This is the quality bar. It is non-negotiable.

> Run `reconstruct` on the main topic. Read the output. Could you make a
> **non-trivial change** — not a typo fix, but a type change, a refactor, a
> new feature — using ONLY that output? Without opening a single source file?

If yes → the compression is lossless. Move to the next module.
If no → identify exactly what's missing. Store it. Test again.

## Three Kinds of Knowledge

Every codebase needs all three. Most seeding efforts only produce the first.

**1. Semantic** — what things do and why
- Module maps, API surfaces, decisions, invariants, data flows, gotchas
- Changes rarely. High shelf life. The backbone of understanding.

**2. Structural** — how things connect
- Coupling profiles: which files access which fields/types, and how
- Access pattern catalogs: clone, compare, map key, format arg, method call
- Type dependency graphs: what traits are required at each connection point
- Changes with every refactor. Short shelf life. **Critical for multi-file changes.**

**3. Procedural** — how to change things
- Refactoring playbooks: step-by-step recipes for recurring change patterns
- Transformation rules: "if you change X, do Y at every site"
- How-tos: operational recipes for common tasks
- Grows with experience. Long shelf life. Prevents reinventing the wheel.

## LLM-Native Format

Every entry you store must be:

- **Atomic**: one file, one concept, one concern
- **Tagged**: at least 2 tags (content type + domain)
- **Source-linked**: `[source: path:line]` when referring to specific code
- **Freshness-annotated**: stored with source links so `check_stale` can detect drift
- **Gap-seeded**: if you can't fully compress something, store what's missing
- **Size-budgeted**: ≤15 lines. If longer, split into atomic sub-entries.
- **Decompressible**: contributes to `reconstruct` producing a complete mental model

Tag conventions:

| Category | Tags |
|----------|------|
| Content type | `module-map`, `api-surface`, `coupling`, `decision`, `invariant`, `change-impact`, `data-flow`, `gotcha`, `pattern`, `playbook`, `transformation`, `how-to`, `gap`, `metrics` |
| Priority | `critical` (invariants, change impacts that prevent data loss) |
| Status | `verified`, `speculative`, `stale` |
| Domain | project-specific: `search`, `storage`, `ffi`, `mcp`, etc. |

## Authority

You have full authority. sudo, SIP disabled, Full Disk Access.
Install tools, build experiments, reverse-engineer binaries, read headers.
You are NOT authorized to modify project source files. Read-only.
The only writes are to amaranthine.

## Research Depth

- **Level 1**: "Does this exist?" — never stop here
- **Level 2**: "How does it work? Failure modes? Edge cases?"
- **Level 3**: "What would I build if this didn't exist?"
- **Level 4**: Full decompressible mental model (required for core modules)

Core modules → Level 4. Glue → Level 2.
If you can't reach Level 3, store a gap entry explaining what's missing.

## Operating Mode

- Spawn 2-3 sub-agents in parallel for independent research threads
- Store findings DURING work, not at the end. Atomic entries > polished summaries.
- Use `batch_store` for groups of 3-8 related entries
- Every amaranthine interaction must advance `reconstruct` output quality

## Phase 0: Load Existing Knowledge

Before reading source:

1. `context(brief: "true")` — see all topics
2. `search("<project>")` — existing knowledge
3. `search(tag: "module-map")` — prior seeding
4. `search(tag: "gotcha")` — known pitfalls
5. `check_stale()` — outdated entries

Rules:
- Don't re-discover what a past session already stored
- Don't contradict without reading WHY an entry says what it says
- Update stale entries with `update_entry` — never create duplicates
- If a topic is well-seeded, focus on gaps and cross-cutting analysis

## Phase 1: Ground Truth

Verify the project's documentation against reality:

```bash
# Baseline counts (adjust extension)
find . -name "*.<EXT>" -not -path "*/test*" -not -path "*/references/*" | wc -l
find . -name "*.<EXT>" -not -path "*/test*" -not -path "*/references/*" -exec wc -l {} + | tail -1
```

Store baseline metrics. Verify every claim in CLAUDE.md / DESIGN.md.
Store every drift found. Stale docs are poison for future sessions.

## Phase 2: Module Map (Semantic)

Read every source file. For each, store a module-map entry:

```
[tags: api-surface, module-map]
[source: <relative/path/to/file>:1]
<filename> (<N> lines) — <one-line purpose>
Types: <struct/enum/class names with key fields>
Pub API:
  <fn(params) -> ReturnType>  // description
  <fn(params) -> ReturnType>  // description
Internal: <private functions worth noting>
Called by: <modules that call into this one>
Uses: <modules this one depends on>
Invariant: <the ONE most critical correctness property>
```

Batch by architectural layer (3-8 entries per `batch_store`):
1. Data layer — storage, formats, serialization
2. Logic layer — algorithms, scoring, processing
3. API layer — external interfaces, protocols, routing
4. Presentation — views, formatting, output
5. Infrastructure — config, utilities, helpers
6. Entry points — main, CLI, server startup

## Phase 3: Structural Coupling

This is what makes multi-file refactors instant instead of painful.

For each **central type** (struct/enum/protocol accessed by 5+ files):

1. Use `codepath` to find all access sites:
   ```
   codepath(pattern: "e.field_name", path: "src/", glob: "*.<ext>")
   ```

2. Categorize each site by access pattern:
   - **clone/copy**: creates owned value from reference
   - **compare**: equality/ordering check, note RHS type
   - **map_key**: used as HashMap/BTreeMap key
   - **format_arg**: interpolated into string/display
   - **method_call**: method invoked on the value
   - **field_access**: sub-field or property read

3. Store coupling profile:
   ```
   [tags: coupling, structural, <type-name>]
   [source: <path-to-type-definition>:1]
   COUPLING: <TypeName>.<field> — <N> sites across <M> files
   Patterns:
     clone: <file1>:<line>, <file2>:<line> (→ .to_string() if type changes)
     compare: <file3>:<line> vs &str (→ needs PartialEq<str>)
     map_key: <file4>:<line> BTreeMap<Type, _> (→ needs Ord + Borrow<str>)
   Transformation: changing from String → NewType requires:
     1. NewType must impl Deref<Target=str> for seamless reads
     2. .clone() sites → .to_string() (N sites)
     3. comparison sites work via Deref (0 changes if PartialEq<str>)
   ```

Focus on the **3-5 most coupled types** — the ones where a field change
cascades across 10+ files. These are the highest-leverage entries.

## Phase 4: Cross-Cutting Analysis (Semantic + Procedural)

### Decisions (tag: `decision`)
For every non-obvious design choice:
```
DECISION: Why X instead of Y — <reasoning>
```
Highest-leverage entries. Without them, future sessions will "improve"
correct design choices and introduce regressions.

### Invariants (tag: `invariant, critical`)
Things that MUST be true or data corrupts / crashes / silent bugs:
```
INVARIANT: <what must hold> [source: <file>:<line>]
```

### Change Impact Maps (tag: `change-impact, critical`)
For every load-bearing file:
```
CHANGE IMPACT: <file> — changing X requires updating A, B, C because...
```

### Data Flow Traces (tag: `data-flow`)
End-to-end traces of key operations:
```
DATA FLOW: <operation> — step1 → step2 → step3
```

### Gotchas (tag: `gotcha`)
```
GOTCHA: <non-obvious behavior that would bite a new session>
```

### Patterns (tag: `pattern`)
```
PATTERN: <recurring code pattern that future sessions should follow>
```

### Transformation Rules (tag: `transformation`)
After analyzing coupling, store what changes are needed for common type changes:
```
TRANSFORMATION: <Type>.<field> String→InternedStr:
  .clone() → .to_string() (15 sites)
  == &str → works via Deref (0 changes)
  BTreeMap key → needs .to_string() (3 sites)
  format!() → works via Display (0 changes)
```

### Refactoring Playbooks (tag: `playbook`)
After completing a multi-file change, store the recipe:
```
PLAYBOOK: Type-level field change
  1. codepath to find all access sites
  2. Categorize by pattern (clone/compare/map/format/method)
  3. Design new type's trait surface to minimize changes
  4. Mechanical replacement per pattern category
  5. Build + verify after each file
```

### How-Tos (tag: `how-to`)
```
HOW-TO: <task> — (1) do X (2) do Y (3) do Z
```

## Phase 5: Deep Analysis

Go beyond structure into behavior:

- **Performance**: hot paths, latency budgets, O(n^2), unnecessary allocs
- **Security**: untrusted inputs, trust boundaries, injection risks
- **Error handling**: propagation consistency, silent failures
- **Coupling**: what changes together, leaky abstractions

## Phase 6: Gap Seeding

**Mandatory. Do not skip.**

End with 5-10 explicit gap entries:
```
GAP: <what's missing and how to investigate>
```

Types of gaps:
1. Modules you couldn't fully compress
2. Undocumented behavior you noticed but couldn't verify
3. Structural coupling you suspect but didn't confirm
4. Data flows you could only partially trace
5. Transformation rules you couldn't fully specify

## Scorecard

| Metric | Count |
|--------|-------|
| Source files read | — |
| Module-map entries | — |
| Coupling profiles | — |
| Decisions documented | — |
| Invariants identified | — |
| Change impact maps | — |
| Data flow traces | — |
| Transformation rules | — |
| Refactoring playbooks | — |
| Gotchas found | — |
| Gaps seeded | — |
| Stale entries updated | — |
| **Total entries stored** | — |

Targets:
- 1 module-map entry per source file
- 1 coupling profile per central type (3-5 minimum)
- 10-20 cross-cutting entries (decisions, invariants, impacts, gotchas)
- 5-10 gap entries
- Zero re-discoveries

---

<!-- PROJECT CONTEXT — Replace below for each project -->

## Project: <NAME>

### Build
```bash
<build commands>
```

### Source Layout
<where source files live, how they're organized, total file count>

### Central Types
<the 3-5 types/structs/classes accessed by the most files — seed these first>

### Topic Naming
- `<project>-project` — architecture, decisions, metrics
- `<project>-<module>` — per-module deep knowledge
- `<project>-gotchas` — pitfalls and surprises
- `<project>-patterns` — conventions and recurring patterns

### Prior Knowledge
<paste `context --brief` results, or "none">

### Known Blockers
<unsolved problems from prior sessions, or "none">

### Available Tools
<project-specific scripts, test harnesses, diagnostic tools>
<system tools: sudo, nm, otool, dtrace, etc.>
