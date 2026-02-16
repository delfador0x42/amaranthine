#!/bin/bash
# PostToolUse hook for Bash: after build commands, remind to store results.
# Only triggers on xcodebuild/cargo/swift build commands.
INPUT=$(cat)

COMMAND=$(echo "$INPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
ti = d.get('tool_input', {})
print(ti.get('command', ''))
" 2>/dev/null)

# Check if this was a build command
case "$COMMAND" in
  *xcodebuild*build*|*cargo\ build*|*swift\ build*|*swiftc\ *)
    echo '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"BUILD COMPLETED. If the build failed with a non-obvious error, store the root cause in amaranthine (topic: build-gotchas). If it succeeded after fixing an issue, store what fixed it."}}'
    ;;
esac
exit 0
