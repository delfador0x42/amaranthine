#!/bin/bash
# PreToolUse (Read): surface amaranthine knowledge when reading iris .swift files.
# Skips if global debounce active (<10s), per-file debounce (<60s), or no-match cached.
INPUT=$(cat)
AMR=/Users/tal/.local/bin/amaranthine
STAMP=/tmp/amaranthine-hook-read.last
GLOBAL=/tmp/amaranthine-hook-global.last
MISS_CACHE=/tmp/amaranthine-miss-cache

# Extract file_path
FP=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tool_input',{}).get('file_path',''))" 2>/dev/null)
[ -z "$FP" ] && exit 0

# Fast-path: only .swift source under iris project
case "$FP" in *.swift) ;; *) exit 0 ;; esac
case "$FP" in */iris/*) ;; *) exit 0 ;; esac
case "$FP" in */build/*|*/scripts/*|*/references/*|*/DerivedData/*|*/test-*|*/.build/*) exit 0 ;; esac

NOW=$(date +%s)

# Global debounce: skip if any hook fired within 10s
if [ -f "$GLOBAL" ]; then
  GLAST=$(cat "$GLOBAL" 2>/dev/null)
  [ $((NOW - ${GLAST:-0})) -lt 10 ] && exit 0
fi

# Per-file debounce: skip if same file within 60s
if [ -f "$STAMP" ]; then
  read -r LAST_TIME LAST_FILE < "$STAMP" 2>/dev/null
  [ $((NOW - ${LAST_TIME:-0})) -lt 60 ] && [ "$LAST_FILE" = "$FP" ] && exit 0
fi

# No-match cache: skip files that never returned results this session
FNAME=$(basename "$FP")
if [ -f "$MISS_CACHE" ] && grep -qF "$FNAME" "$MISS_CACHE"; then
  exit 0
fi

echo "$NOW $FP" > "$STAMP"

RESULTS=$(python3 -c "
import re, os, subprocess, sys

fp = sys.argv[1]
fname = os.path.basename(fp)
name = os.path.splitext(fname)[0].split('+')[0]
parts = re.findall(r'[A-Z]+(?=[A-Z][a-z])|[A-Z][a-z]*|[a-z]+|[A-Z]+', name)
parent = os.path.basename(os.path.dirname(fp))

queries = []
if len(name) >= 3:
    queries.append(name)
if len(parts) >= 2:
    sig = ' '.join(p for p in parts if len(p) >= 3)
    if sig: queries.append(sig)
if len(parent) >= 3 and parts:
    biggest = max(parts, key=len)
    if len(biggest) >= 3:
        queries.append(parent + ' ' + biggest)

amr = '$AMR'
for q in queries:
    if not q.strip(): continue
    r = subprocess.run([amr, '--plain', 'search', q, '--brief', '--limit', '2'],
                       capture_output=True, text=True, timeout=5)
    if r.returncode == 0 and '[' in r.stdout and 'no matches' not in r.stdout:
        print(r.stdout.rstrip())
        sys.exit(0)
" "$FP" 2>/dev/null)

# No results — cache as a miss for rest of session
if [ -z "$RESULTS" ]; then
  echo "$FNAME" >> "$MISS_CACHE"
  exit 0
fi

# Set global debounce — suppress subsequent hooks for 10s
echo "$NOW" > "$GLOBAL"

ESCAPED=$(echo "$RESULTS" | python3 -c "import sys,json; print(json.dumps('Prior knowledge for this file:\n' + sys.stdin.read()))")
echo "{\"hookSpecificOutput\":{\"additionalContext\":$ESCAPED}}"
exit 0
