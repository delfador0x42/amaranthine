#!/bin/bash
# Stop: remind to store findings before conversation ends.
# Debounced at 120s to avoid repeated noise.
STAMP=/tmp/amaranthine-hook-stop.last
NOW=$(date +%s)
if [ -f "$STAMP" ]; then
  LAST=$(cat "$STAMP" 2>/dev/null)
  [ $((NOW - ${LAST:-0})) -lt 120 ] && exit 0
fi
echo "$NOW" > "$STAMP"

echo '{"hookSpecificOutput":{"additionalContext":"STOPPING: Store any non-obvious findings in amaranthine before ending."}}'
exit 0
