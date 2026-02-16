#!/bin/bash
# SubagentStart hook: inject amaranthine search context into subagents.
echo '{"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"AMARANTHINE KNOWLEDGE STORE: You have access to amaranthine MCP tools. BEFORE starting work, call mcp__amaranthine__search_medium with keywords relevant to your task to check for prior findings. Topics: iris-scanners, iris-project, iris-engine, iris-network, build-gotchas, rust-ffi, amaranthine-gotchas, amaranthine-improvements."}}'
exit 0
