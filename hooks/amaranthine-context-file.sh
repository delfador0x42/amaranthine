#!/bin/bash
# PreToolUse hook for Edit|Write: search amaranthine for knowledge about the file being edited.
# Surfaces relevant entries before you modify a file.
INPUT=$(cat)

AMR=/Users/tal/.local/bin/amaranthine

RESULTS=$(echo "$INPUT" | python3 -c "
import sys, json, subprocess, re

d = json.load(sys.stdin)
ti = d.get('tool_input', {})
fp = ti.get('file_path', '')
if not fp:
    sys.exit(0)

# Extract filename without extension
import os
fname = os.path.basename(fp)
name = os.path.splitext(fname)[0]

# Strip common suffixes/prefixes
name = name.split('+')[0]  # TCCMonitor+Helpers -> TCCMonitor

# Split camelCase: TCCMonitor -> ['TCC', 'Monitor']
parts = re.findall(r'[A-Z]+(?=[A-Z][a-z])|[A-Z][a-z]*|[a-z]+|[A-Z]+', name)
# Also get parent dir name
parent = os.path.basename(os.path.dirname(fp))

# Build search queries to try
queries = []

# Full name first (best match)
if len(name) >= 3:
    queries.append(name)

# Name parts joined (e.g. 'TCC Monitor')
if len(parts) >= 2:
    queries.append(' '.join(p for p in parts if len(p) >= 3))

# Parent + significant part
if len(parent) >= 3 and len(parts) >= 1:
    biggest = max(parts, key=len)
    if len(biggest) >= 3:
        queries.append(f'{parent} {biggest}')

amr = '$AMR'
for q in queries:
    if not q.strip():
        continue
    r = subprocess.run([amr, '--plain', 'search', q, '--brief', '--limit', '3'],
                       capture_output=True, text=True, timeout=5)
    if r.returncode == 0 and '[' in r.stdout and 'no matches' not in r.stdout:
        print(r.stdout.rstrip())
        sys.exit(0)
" 2>/dev/null)

[ -z "$RESULTS" ] && exit 0

ESCAPED=$(echo "$RESULTS" | python3 -c "import sys,json; print(json.dumps('Amaranthine knowledge for this file:\n' + sys.stdin.read()))")
echo "{\"hookSpecificOutput\":{\"additionalContext\":$ESCAPED}}"
exit 0
