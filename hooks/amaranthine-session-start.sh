#!/bin/bash
# SessionStart: load amaranthine context + set session timestamp.
AMR=/Users/tal/.local/bin/amaranthine

# Write session start timestamp for session-scoped queries
date +%s > /tmp/amaranthine-session-ts

# Clear caches from previous session
rm -f /tmp/amaranthine-miss-cache
rm -f /tmp/amaranthine-hook-postedit.seen

# Clear all hook debounce stamps
rm -f /tmp/amaranthine-hook-global.last
rm -f /tmp/amaranthine-hook-prompt.last
rm -f /tmp/amaranthine-hook-file.last
rm -f /tmp/amaranthine-hook-read.last
rm -f /tmp/amaranthine-hook-stop.last

# Show recent entries (last 3 days) instead of topic list (already in MEMORY.md)
CONTEXT=$("$AMR" --plain recent 3 2>/dev/null)
if [ -n "$CONTEXT" ]; then
  ESCAPED=$(echo "$CONTEXT" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))")
  echo "{\"hookSpecificOutput\":{\"additionalContext\":$ESCAPED}}"
fi
exit 0
