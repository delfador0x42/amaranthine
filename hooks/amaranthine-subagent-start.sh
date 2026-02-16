#!/bin/bash
# SubagentStart hook: inject amaranthine search context into every subagent.
# Tells the subagent to search amaranthine and store findings.
echo '{"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"AMARANTHINE KNOWLEDGE STORE: You have access to amaranthine MCP tools for cross-session knowledge. BEFORE starting work, call mcp__amaranthine__search with keywords relevant to your task. BEFORE returning results, call mcp__amaranthine__store for any non-obvious findings (bug fixes, API discoveries, gotchas, architectural insights). Use small atomic entries. Topic names: iris-scanners, iris-engine, iris-network, iris-gotchas, build-gotchas, rust-ffi, crash-analysis, iris-patterns, iris-project, or create new ones."}}'
exit 0
