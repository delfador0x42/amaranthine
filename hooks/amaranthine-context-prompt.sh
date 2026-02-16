#!/bin/bash
# UserPromptSubmit: search amaranthine for knowledge relevant to user's prompt.
# Sets global debounce stamp so read/file hooks skip for 10s.
STAMP=/tmp/amaranthine-hook-prompt.last
GLOBAL=/tmp/amaranthine-hook-global.last
NOW=$(date +%s)

# Per-hook debounce: skip if fired <30s ago
if [ -f "$STAMP" ]; then
  LAST=$(cat "$STAMP" 2>/dev/null)
  [ $((NOW - ${LAST:-0})) -lt 30 ] && exit 0
fi
echo "$NOW" > "$STAMP"

INPUT=$(cat)
AMR=/Users/tal/.local/bin/amaranthine

PROMPT=$(echo "$INPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(d.get('prompt', ''))
" 2>/dev/null)

[ -z "$PROMPT" ] && exit 0

# Set global stamp — read/file hooks will skip for 10s
echo "$NOW" > "$GLOBAL"

# Extract keywords and do progressive search
RESULTS=$(echo "$PROMPT" | python3 -c "
import sys, subprocess

stops = {'the','a','an','is','it','its','can','do','does','did','fix','add','make',
         'update','change','modify','create','delete','remove','get','set','use',
         'this','that','these','those','what','how','why','when','where','which',
         'for','with','from','into','about','then','also','just','now','here',
         'please','thanks','help','want','need','should','would','could','lets',
         'let','and','but','or','not','if','so','to','of','in','on','at','by',
         'my','me','we','us','you','your','our','im','ive','youre','were','hes',
         'she','they','them','all','any','some','new','old','try','run','see',
         'look','check','tell','show','give','go','know','think','like','file',
         'code','sure','yeah','yes','hmm','ok','okay','alright','hey','hi',
         'hello','bug','error','issue','problem','work','working','implement',
         'investigate','review','explore','analyze','understand','research','build',
         'test','debug','refactor','optimize','improve','write','read','find','search',
         'figure','out','about','into','something','everything','anything','nothing',
         'keep','going','continue','hell','rock','brother','man','hahahaha','haha',
         'lol','cool','nice','great','awesome','perfect','good','done','right','totally'}

words = sys.stdin.read().lower().split()
kw = []
seen = set()
for w in words:
    w = w.strip('.,!?:;()[]{}' + chr(34) + chr(39))
    if len(w) >= 3 and w not in stops and w not in seen:
        seen.add(w)
        kw.append(w)
    if len(kw) >= 5:
        break

if not kw:
    sys.exit(0)

amr = '$AMR'
for drop in range(min(len(kw), 3)):
    query = ' '.join(kw[:len(kw)-drop])
    if not query:
        break
    r = subprocess.run([amr, '--plain', 'search', query, '--brief', '--limit', '5'],
                       capture_output=True, text=True, timeout=5)
    if r.returncode == 0 and '[' in r.stdout and 'no matches' not in r.stdout:
        print(r.stdout.rstrip())
        sys.exit(0)
" 2>/dev/null)

[ -z "$RESULTS" ] && exit 0

ESCAPED=$(echo "$RESULTS" | python3 -c "import sys,json; print(json.dumps('Amaranthine knowledge for this task:\n' + sys.stdin.read()))")
echo "{\"hookSpecificOutput\":{\"additionalContext\":$ESCAPED}}"
exit 0
