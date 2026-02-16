#!/bin/bash
# PreToolUse hook for Read: surface amaranthine knowledge when reading iris source files.
# Stricter than Edit|Write hook — only fires for .swift files in iris source directories,
# skipping build artifacts, scripts, references, and non-source files.
INPUT=$(cat)
AMR=/Users/tal/.local/bin/amaranthine

RESULTS=$(echo "$INPUT" | python3 -c "
import sys, json, subprocess, re, os

d = json.load(sys.stdin)
ti = d.get('tool_input', {})
fp = ti.get('file_path', '')
if not fp:
    sys.exit(0)

# Only .swift source files
if not fp.endswith('.swift'):
    sys.exit(0)

# Must be in iris source directories (not build, scripts, references, tests)
skip = ['/build/', '/scripts/', '/references/', '/DerivedData/', '/test-', '/.build/']
if any(s in fp for s in skip):
    sys.exit(0)

# Must be under the iris project
if '/iris/' not in fp:
    sys.exit(0)

fname = os.path.basename(fp)
name = os.path.splitext(fname)[0]
# Strip extension category (PersistenceScanner+Shell -> PersistenceScanner)
name = name.split('+')[0]

# Split camelCase into parts
parts = re.findall(r'[A-Z]+(?=[A-Z][a-z])|[A-Z][a-z]*|[a-z]+|[A-Z]+', name)
parent = os.path.basename(os.path.dirname(fp))

queries = []
if len(name) >= 3:
    queries.append(name)
if len(parts) >= 2:
    sig = ' '.join(p for p in parts if len(p) >= 3)
    if sig:
        queries.append(sig)
if len(parent) >= 3 and parts:
    biggest = max(parts, key=len)
    if len(biggest) >= 3:
        queries.append(parent + ' ' + biggest)

amr = '$AMR'
for q in queries:
    if not q.strip():
        continue
    r = subprocess.run([amr, '--plain', 'search', q, '--brief', '--limit', '2'],
                       capture_output=True, text=True, timeout=5)
    if r.returncode == 0 and '[' in r.stdout and 'no matches' not in r.stdout:
        print(r.stdout.rstrip())
        sys.exit(0)
" 2>/dev/null)

[ -z "$RESULTS" ] && exit 0

ESCAPED=$(echo "$RESULTS" | python3 -c "import sys,json; print(json.dumps('Prior knowledge for this file:\n' + sys.stdin.read()))")
echo "{\"hookSpecificOutput\":{\"additionalContext\":$ESCAPED}}"
exit 0
