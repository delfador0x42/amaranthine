use crate::json::Value;
use std::sync::{Arc, Mutex};

static TOOL_CACHE: Mutex<Option<Arc<str>>> = Mutex::new(None);

/// Return pre-serialized JSON for the tools/list result object.
/// Cached after first call — Arc avoids cloning the ~15KB JSON string.
pub fn tool_list_json() -> Arc<str> {
    if let Ok(guard) = TOOL_CACHE.lock() {
        if let Some(cached) = &*guard { return Arc::clone(cached); }
    }
    let result = Value::Obj(vec![("tools".into(), tool_list())]);
    let json: Arc<str> = result.to_string().into();
    if let Ok(mut guard) = TOOL_CACHE.lock() { *guard = Some(Arc::clone(&json)); }
    json
}

pub fn tool(name: &str, desc: &str, req: &[&str], props: &[(&str, &str, &str)]) -> Value {
    Value::Obj(vec![
        ("name".into(), Value::Str(name.into())),
        ("description".into(), Value::Str(desc.into())),
        ("inputSchema".into(), Value::Obj(vec![
            ("type".into(), Value::Str("object".into())),
            ("properties".into(), Value::Obj(props.iter().map(|(n, t, d)| {
                ((*n).into(), Value::Obj(vec![
                    ("type".into(), Value::Str((*t).into())),
                    ("description".into(), Value::Str((*d).into())),
                ]))
            }).collect())),
            ("required".into(), Value::Arr(
                req.iter().map(|r| Value::Str((*r).into())).collect()
            )),
        ])),
    ])
}

fn batch_tool() -> Value {
    let entry_schema = Value::Obj(vec![
        ("type".into(), Value::Str("object".into())),
        ("properties".into(), Value::Obj(vec![
            ("topic".into(), Value::Obj(vec![
                ("type".into(), Value::Str("string".into())),
                ("description".into(), Value::Str("Topic name".into())),
            ])),
            ("text".into(), Value::Obj(vec![
                ("type".into(), Value::Str("string".into())),
                ("description".into(), Value::Str("Entry content".into())),
            ])),
            ("tags".into(), Value::Obj(vec![
                ("type".into(), Value::Str("string".into())),
                ("description".into(), Value::Str("Comma-separated tags".into())),
            ])),
            ("source".into(), Value::Obj(vec![
                ("type".into(), Value::Str("string".into())),
                ("description".into(), Value::Str("Source file: path/to/file:line for staleness detection".into())),
            ])),
        ])),
        ("required".into(), Value::Arr(vec![
            Value::Str("topic".into()), Value::Str("text".into()),
        ])),
    ]);

    Value::Obj(vec![
        ("name".into(), Value::Str("batch".into())),
        ("description".into(), Value::Str(
            "Store multiple entries in one call. Each entry: {topic, text, tags?}. Faster than sequential store calls.".into()
        )),
        ("inputSchema".into(), Value::Obj(vec![
            ("type".into(), Value::Str("object".into())),
            ("properties".into(), Value::Obj(vec![
                ("entries".into(), Value::Obj(vec![
                    ("type".into(), Value::Str("array".into())),
                    ("items".into(), entry_schema),
                    ("description".into(), Value::Str("Array of entries to store".into())),
                ])),
                ("verbose".into(), Value::Obj(vec![
                    ("type".into(), Value::Str("string".into())),
                    ("description".into(), Value::Str(
                        "Set to 'true' for per-entry details (default: terse count only)".into()
                    )),
                ])),
            ])),
            ("required".into(), Value::Arr(vec![Value::Str("entries".into())])),
        ])),
    ])
}

const SEARCH_FILTER_PROPS: &[(&str, &str, &str)] = &[
    ("limit", "string", "Max results to return (default: unlimited)"),
    ("after", "string", "Only entries on/after date (YYYY-MM-DD or 'today'/'yesterday'/'this-week')"),
    ("before", "string", "Only entries on/before date (YYYY-MM-DD or 'today'/'yesterday')"),
    ("days", "string", "Number of days (shortcut for after=N-days-ago)"),
    ("hours", "string", "Number of hours (overrides days)"),
    ("tag", "string", "Only entries with this tag"),
    ("topic", "string", "Limit search to a single topic"),
    ("mode", "string", "Search mode: 'and' (default, all terms must match) or 'or' (any term matches)"),
];

pub fn tool_list() -> Value {
    let search_props: Vec<(&str, &str, &str)> = [
        ("query", "string", "Search query"),
        ("detail", "string", "Result detail level: 'full' (complete entry), 'medium' (default, 2 lines), 'brief' (topic+first line), 'count' (match count only), 'topics' (hits per topic), 'grouped' (results by topic), or 'index' (binary index search)"),
    ].into_iter()
        .chain(SEARCH_FILTER_PROPS.iter().copied())
        .collect();

    Value::Arr(vec![
        // === PRIMARY TOOLS (use these most) ===
        tool("store", "Store a timestamped knowledge entry under a topic. Warns on duplicate content.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Entry content"),
              ("tags", "string", "Comma-separated tags (e.g. 'bug,p0,iris')"),
              ("force", "string", "Set to 'true' to bypass duplicate detection"),
              ("source", "string", "Source file reference: 'path/to/file:line'. Enables staleness detection."),
              ("terse", "string", "Set to 'true' for minimal response (just first line)"),
              ("confidence", "string", "Confidence level 0.0-1.0 (default: 1.0). Affects search ranking."),
              ("links", "string", "Space-separated references: 'topic:index topic:index'. Creates narrative links.")]),
        batch_tool(),
        tool("search", "Search all knowledge files (case-insensitive). Splits CamelCase/snake_case. Falls back to OR when AND finds nothing. Use detail param: 'full' (complete entry), 'medium' (default, 2 lines), 'brief' (topic+first line), 'count' (match count only), 'topics' (hits per topic).",
            &[], &search_props),
        tool("brief", "One-shot compressed briefing for a topic or pattern. Primary way to load a mental model. Default output is a ~15-line summary; use detail='scan' for category one-liners, detail='full' for complete entries. Use since=N for entries from last N hours only. Supports glob patterns like 'iris-*' for multi-topic views. Without query: session start briefing (activity-weighted topics + velocity).",
            &[],
            &[("query", "string", "Topic, keyword, or glob pattern (e.g. 'iris-*', 'engine', 'amaranthine-codebase')"),
              ("detail", "string", "Output tier: 'summary' (default, ~15 lines), 'scan' (category one-liners), 'full' (complete entries)"),
              ("since", "string", "Only entries from last N hours (e.g. '24' for last day, '48' for 2 days)"),
              ("focus", "string", "Comma-separated category names to show (e.g. 'gotchas,invariants'). Only matching categories appear in output."),
              ("compact", "string", "Set to 'true' for compact meta-briefing (top 5 topics only)")]),
        tool("read", "Read the full contents of a specific topic file.",
            &["topic"],
            &[("topic", "string", "Topic name")]),

        // === WRITE TOOLS ===
        tool("append", "Add text to the last entry in a topic (no new timestamp). Use when adding related info to a recent entry. Pass index/match_str/tag to target a specific entry instead.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Text to append"),
              ("index", "string", "Entry index number (from entries)"),
              ("match_str", "string", "Substring to find the entry to append to"),
              ("tag", "string", "Append to most recent entry with this tag")]),
        tool("delete", "Delete entries or entire topic. Use index/match_str to target specific entries, or all=true for entire topic.",
            &["topic"],
            &[("topic", "string", "Topic name"),
              ("index", "string", "Delete entry by index number (from entries)"),
              ("match_str", "string", "Delete entry matching this substring"),
              ("all", "string", "Set to 'true' to delete entire topic")]),
        tool("revise", "Overwrite an existing entry's text (keeps timestamp). Adds [modified] marker.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Substring to find the entry to revise"),
              ("index", "string", "Entry index number (from entries)"),
              ("text", "string", "Replacement text for the entry")]),
        tool("tag", "Add or remove tags on an existing entry.",
            &["topic", "tags"],
            &[("topic", "string", "Topic name"),
              ("index", "string", "Entry index number (from entries)"),
              ("match_str", "string", "Substring to find the entry"),
              ("tags", "string", "Comma-separated tags to add"),
              ("remove", "string", "Comma-separated tags to remove")]),
        tool("rename", "Rename a topic. All entries preserved.",
            &["topic", "new_name"],
            &[("topic", "string", "Current topic name"),
              ("new_name", "string", "New topic name")]),
        tool("merge", "Merge all entries from one topic into another. Source topic is deleted after merge.",
            &["from", "into"],
            &[("from", "string", "Source topic to merge FROM (will be deleted)"),
              ("into", "string", "Target topic to merge INTO")]),

        // === BROWSE TOOLS ===
        tool("topics", "List all topic files with entry and line counts.",
            &[], &[]),
        tool("recent", "Show entries from last N days (or hours) across all topics.",
            &[],
            &[("days", "string", "Number of days (default: 7)"),
              ("hours", "string", "Number of hours (overrides days for finer granularity)")]),
        tool("entries", "List entries in a topic with index numbers. Use before delete/revise/tag. Pass index to fetch a single entry.",
            &["topic"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Only show entries matching this substring"),
              ("index", "string", "Fetch a single entry by index (0-based)")]),
        tool("stats", "Show stats: topic count, entry count, date range, tag count. Use detail='tags' for all tags with counts, detail='index' for binary index health.",
            &[],
            &[("detail", "string", "Output: default (overview), 'tags' (all tags with counts), 'index' (binary index stats)")]),

        // === ANALYSIS TOOLS ===
        tool("stale", "Scan entries with [source:] metadata and report which source files changed. Use refresh=true to see stale entries alongside current source code.",
            &[],
            &[("refresh", "string", "Set to 'true' to show stale entries + current source side-by-side")]),
        tool("xref", "Find cross-references: entries in other topics that mention this topic.",
            &["topic"],
            &[("topic", "string", "Topic to find references for")]),
        tool("graph", "Topic dependency graph: which topics reference which. Shows bidirectional edges sorted by connectivity.",
            &[],
            &[("focus", "string", "Glob pattern to filter topics (e.g. 'iris-*')")]),
        tool("trace", "Analyze a codebase: trace function callers/callees (callgraph), find access sites (codepath), map architecture (reverse), find core vs dead code (core), find similar/thin files (simplify), debug crashes (crash), or profile perf antipatterns (perf).",
            &["path"],
            &[("path", "string", "Codebase directory to search"),
              ("pattern", "string", "Function name, search string, or crash/stack trace text (required for callgraph/codepath/crash)"),
              ("mode", "string", "Analysis type: 'callgraph' (default), 'codepath', 'reverse', 'core', 'simplify', 'crash', 'perf'"),
              ("glob", "string", "File filter suffix (default: *.rs)"),
              ("depth", "string", "Recursion depth for callgraph/perf (default: 2, max: 5)"),
              ("direction", "string", "callgraph direction: callers|callees|both (default: both)"),
              ("context", "string", "Lines of context for codepath (default: 2)"),
              ("entry", "string", "Entry point function for core/perf mode (default for core: 'main|run')"),
              ("store_topic", "string", "If set, store results under this topic"),
              ("tags", "string", "Tags for stored entry")]),

        // === MAINTENANCE TOOLS ===
        tool("compact", "Find and merge duplicate entries within a topic. Use log=true to rewrite data.log. Use mode='migrate' to fix entries without timestamps.",
            &[],
            &[("topic", "string", "Topic to compact (omit to scan all)"),
              ("apply", "string", "Set to 'true' to actually apply (default: dry run)"),
              ("log", "string", "Set to 'true' to compact the data.log (reclaim deleted space)"),
              ("mode", "string", "Operation: 'dedup' (default) or 'migrate' (fix timestamps)")]),
        tool("prune", "Flag stale topics (no entries in N days).",
            &[],
            &[("days", "string", "Stale threshold in days (default: 30)")]),
        tool("export", "Export all topics as structured JSON for backup.",
            &[], &[]),
        tool("import", "Import topics from JSON (merges with existing data).",
            &["json"],
            &[("json", "string", "JSON string to import")]),
        tool("reindex", "Rebuild the binary inverted index from all topic files.",
            &[], &[]),
        tool("session", "Show what was stored this session.",
            &[], &[]),
        tool("_reload", "Re-exec the server binary to pick up code changes.",
            &[], &[]),
    ])
}
