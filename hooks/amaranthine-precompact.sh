#!/bin/bash
# PreCompact hook: context is about to be compressed.
# This is the critical moment to persist session knowledge.
echo '{"decision":"allow","reason":"CRITICAL: Context compaction imminent. Before continuing, you MUST store all significant findings from this session to amaranthine using the MCP tools (store, search_brief). Focus on: bug fixes with root causes, architectural decisions, API discoveries, gotchas. Check existing topics first to avoid duplicates. Then continue working."}'
exit 0
