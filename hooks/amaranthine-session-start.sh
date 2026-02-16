#!/bin/bash
# SessionStart hook: load amaranthine context at session start.
# Outputs additionalContext that Claude sees as system context.
CONTEXT=$(/Users/tal/wudan/dojo/crash3/amaranthine/target/release/amaranthine --plain -d /Users/tal/.amaranthine context --brief 2>/dev/null)
if [ -n "$CONTEXT" ]; then
  # Escape for JSON
  ESCAPED=$(echo "$CONTEXT" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))")
  echo "{\"hookSpecificOutput\":{\"additionalContext\":$ESCAPED}}"
fi
exit 0
