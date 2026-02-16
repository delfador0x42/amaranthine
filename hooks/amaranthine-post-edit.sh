#!/bin/bash
# PostToolUse (Edit|Write): remind about potentially stale amaranthine entries.
# Only fires once per file per session. Only iris .swift files.
INPUT=$(cat)
AMR=/Users/tal/.local/bin/amaranthine
SEEN=/tmp/amaranthine-hook-postedit.seen

# Extract file_path
FP=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tool_input',{}).get('file_path',''))" 2>/dev/null)
[ -z "$FP" ] && exit 0

# Only iris .swift files
case "$FP" in *.swift) ;; *) exit 0 ;; esac
case "$FP" in */iris/*) ;; *) exit 0 ;; esac

# Once per file per session
if [ -f "$SEEN" ] && grep -qF "$FP" "$SEEN"; then
  exit 0
fi
echo "$FP" >> "$SEEN"

# Extract base name for search (e.g. "BinaryIntegrityScanner" from "BinaryIntegrityScanner.swift")
FNAME=$(basename "$FP" .swift | sed 's/+.*//')

# Quick count — skip if no entries exist
COUNT=$("$AMR" --plain search_count "$FNAME" 2>/dev/null)
case "$COUNT" in
  *"0 match"*|*"no match"*|"") exit 0 ;;
esac

MSG="You modified $FNAME which has amaranthine entries. Update or delete stale entries if needed."
ESCAPED=$(python3 -c "import json; print(json.dumps('$MSG'))")
echo "{\"hookSpecificOutput\":{\"additionalContext\":$ESCAPED}}"
exit 0
