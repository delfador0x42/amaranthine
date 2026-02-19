use crate::json::Value;

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

fn batch_store_tool() -> Value {
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
        ("name".into(), Value::Str("batch_store".into())),
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
    ("tag", "string", "Only entries with this tag"),
    ("topic", "string", "Limit search to a single topic"),
    ("mode", "string", "Search mode: 'and' (default, all terms must match) or 'or' (any term matches)"),
];

pub fn tool_list() -> Value {
    let search_props: Vec<(&str, &str, &str)> = [
        ("query", "string", "Search query"),
        ("detail", "string", "Result detail level: 'full', 'medium' (default), or 'brief'"),
    ].into_iter()
        .chain(SEARCH_FILTER_PROPS.iter().copied())
        .collect();

    let search_count_props: Vec<(&str, &str, &str)> = std::iter::once(("query", "string", "Search query"))
        .chain(SEARCH_FILTER_PROPS.iter().copied().filter(|(n, _, _)| *n != "limit"))
        .collect();

    Value::Arr(vec![
        tool("store", "Store a timestamped knowledge entry under a topic. Warns on duplicate content.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Entry content"),
              ("tags", "string", "Comma-separated tags (e.g. 'bug,p0,iris')"),
              ("force", "string", "Set to 'true' to bypass duplicate detection"),
              ("source", "string", "Source file reference: 'path/to/file:line'. Enables staleness detection."),
              ("terse", "string", "Set to 'true' for minimal response (just first line)")]),
        tool("append", "Add text to the last entry in a topic (no new timestamp). Use when adding related info to a recent entry.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("text", "string", "Text to append")]),
        batch_store_tool(),
        tool("search", "Search all knowledge files (case-insensitive). Splits CamelCase/snake_case. Falls back to OR when AND finds nothing.",
            &[], &search_props),
        tool("search_brief", "Quick search: just topic names + first matching line per hit",
            &[], &search_props),
        tool("search_medium", "Medium search: topic + timestamp + first 2 content lines per hit. Between brief and full.",
            &[], &search_props),
        tool("search_count", "Count matching sections without returning content. Fast way to gauge query scope.",
            &[], &search_count_props),
        tool("search_topics", "Show which topics matched and how many hits per topic. Best first step before deep search.",
            &[], &search_count_props),
        tool("context", "Session briefing: topics + recent entries (7 days) + optional search",
            &[],
            &[("query", "string", "Optional search query"),
              ("brief", "string", "Set to 'true' for compact mode (topics only, no recent)")]),
        tool("topics", "List all topic files with entry and line counts",
            &[], &[]),
        tool("recent", "Show entries from last N days (or hours) across all topics",
            &[],
            &[("days", "string", "Number of days (default: 7)"),
              ("hours", "string", "Number of hours (overrides days for finer granularity)")]),
        tool("delete_entry", "Remove the most recent entry from a topic",
            &["topic"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Delete entry matching this substring instead of last"),
              ("index", "string", "Delete entry by index number (from list_entries)")]),
        tool("delete_topic", "Delete an entire topic and all its entries",
            &["topic"],
            &[("topic", "string", "Topic name")]),
        tool("append_entry", "Add text to an existing entry found by substring match, index, or tag (keeps timestamp, preserves body)",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Substring to find the entry to append to"),
              ("index", "string", "Entry index number (from list_entries)"),
              ("tag", "string", "Append to most recent entry with this tag"),
              ("text", "string", "Text to append to the entry")]),
        tool("update_entry", "Overwrite an existing entry's text (keeps timestamp). Adds [modified] marker.",
            &["topic", "text"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Substring to find the entry to update"),
              ("index", "string", "Entry index number (from list_entries)"),
              ("text", "string", "Replacement text for the entry")]),
        tool("read_topic", "Read the full contents of a specific topic file",
            &["topic"],
            &[("topic", "string", "Topic name")]),
        tool("digest", "Compact summary of all topics (one bullet per entry)",
            &[], &[]),
        tool("list_tags", "List all tags used across all topics with counts",
            &[], &[]),
        tool("stats", "Show stats: topic count, entry count, date range, tag count",
            &[], &[]),
        tool("list_entries", "List entries in a topic with index numbers. For bulk review before delete.",
            &["topic"],
            &[("topic", "string", "Topic name"),
              ("match_str", "string", "Only show entries matching this substring")]),
        tool("prune", "Flag stale topics (no entries in N days). For identifying outdated knowledge.",
            &[],
            &[("days", "string", "Stale threshold in days (default: 30)")]),
        tool("compact", "Find and merge duplicate entries within a topic. Without topic, scans all topics.",
            &[],
            &[("topic", "string", "Topic to compact (omit to scan all)"),
              ("apply", "string", "Set to 'true' to actually merge (default: dry run)")]),
        tool("export", "Export all topics as structured JSON for backup or migration.",
            &[], &[]),
        tool("import", "Import topics from JSON (merges with existing data).",
            &["json"],
            &[("json", "string", "JSON string to import")]),
        tool("xref", "Find cross-references: entries in other topics that mention this topic.",
            &["topic"],
            &[("topic", "string", "Topic to find references for")]),
        tool("migrate", "Find and fix entries without proper timestamps.",
            &[],
            &[("apply", "string", "Set to 'true' to backfill timestamps (default: dry run)")]),
        tool("get_entry", "Fetch a single entry by topic and index number. Use after list_entries to read specific entries.",
            &["topic", "index"],
            &[("topic", "string", "Topic name"),
              ("index", "string", "Entry index number (0-based, from list_entries)")]),
        tool("rename_topic", "Rename a topic (moves the file). All entries preserved.",
            &["topic", "new_name"],
            &[("topic", "string", "Current topic name"),
              ("new_name", "string", "New topic name")]),
        tool("tag_entry", "Add or remove tags on an existing entry. Use to mark entries as superseded or add missing tags.",
            &["topic", "tags"],
            &[("topic", "string", "Topic name"),
              ("index", "string", "Entry index number (from list_entries)"),
              ("match_str", "string", "Substring to find the entry"),
              ("tags", "string", "Comma-separated tags to add"),
              ("remove", "string", "Comma-separated tags to remove")]),
        tool("rebuild_index", "Rebuild the binary inverted index from all topic files. Enables fast index_search.",
            &[], &[]),
        tool("index_stats", "Show binary index and cache statistics.",
            &[], &[]),
        tool("index_search", "Search using the binary inverted index (~200ns per query). Requires rebuild_index first.",
            &["query"],
            &[("query", "string", "Search query"),
              ("limit", "string", "Max results (default: 10)")]),
        tool("session", "Show what was stored this session. Tracks all store/batch_store calls since server started.",
            &[], &[]),
        tool("search_entity", "Search across all topics, results grouped by topic. Shows the full picture per-topic instead of flat BM25 ranking.",
            &["query"],
            &[("query", "string", "Search query"),
              ("limit", "string", "Max results per topic (default: 5)")]),
        tool("reconstruct", "One-shot compressed briefing: collects + compresses entries (dedup, temporal chains), groups by semantic tag, adds topic map + cross-refs + source pointers + freshness + gap detection. Use for cold-start codebase understanding.",
            &["query"],
            &[("query", "string", "Module or component name to reconstruct (e.g. 'endpoint', 'engine')")]),
        tool("dep_graph", "Topic dependency graph: which topics reference which. Shows bidirectional edges sorted by connectivity.",
            &[], &[]),
        tool("check_stale", "Scan all entries with [source:] metadata and report which source files have been modified since the entry was written.",
            &[], &[]),
        tool("merge_topics", "Merge all entries from one topic into another. Source topic is deleted after merge.",
            &["from", "into"],
            &[("from", "string", "Source topic to merge FROM (will be deleted)"),
              ("into", "string", "Target topic to merge INTO")]),
        tool("_reload", "Re-exec the server binary to pick up code changes. Sends tools/list_changed notification after reload.",
            &[], &[]),
    ])
}
