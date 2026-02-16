#!/bin/bash
# SubagentStop hook: check if subagent stored findings before returning.
# Uses exit code 2 to block if the subagent did significant work.
INPUT=$(cat)

# Extract the subagent type to avoid blocking trivial agents
AGENT_TYPE=$(echo "$INPUT" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('agent_type',''))" 2>/dev/null)

# Only prompt for research-heavy agent types
case "$AGENT_TYPE" in
  Explore|general-purpose|Plan)
    echo '{"decision":"block","reason":"Before returning: Did you store any non-obvious findings in amaranthine? If you discovered gotchas, API behavior, architecture insights, or solutions — store them now with mcp__amaranthine__store. If nothing worth storing, say so and proceed."}'
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
