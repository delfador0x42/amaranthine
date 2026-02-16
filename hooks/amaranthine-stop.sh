#!/bin/bash
# Stop hook: remind Claude to store findings before finishing.
# Must check stop_hook_active to prevent infinite loops.
INPUT=$(cat)

STOP_ACTIVE=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('stop_hook_active', False))" 2>/dev/null)

if [ "$STOP_ACTIVE" = "True" ]; then
  exit 0
fi

echo '{"decision":"block","reason":"BEFORE STOPPING: Review what you just did. If you discovered anything non-obvious (bug fix, gotcha, architectural insight, build issue, API behavior), store it NOW in amaranthine with an atomic entry. If nothing worth storing, proceed."}'
exit 0
