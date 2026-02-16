#!/bin/bash
# PostToolUse (Bash): after build commands, remind to store results.
INPUT=$(cat)

# Extract command — lightweight grep for the common pattern
COMMAND=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tool_input',{}).get('command',''))" 2>/dev/null)

case "$COMMAND" in
  *xcodebuild*build*|*cargo\ build*|*swift\ build*|*swiftc\ *)
    echo '{"hookSpecificOutput":{"additionalContext":"BUILD COMPLETED. If the build failed with a non-obvious error, store the root cause in amaranthine (topic: build-gotchas). If it succeeded after fixing an issue, store what fixed it."}}'
    ;;
esac
exit 0
