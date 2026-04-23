#!/bin/bash
# Claude Code PostToolUse hook installed by `rune setup`.
#
# Reads Claude's PostToolUse JSON from stdin, checks if the file is a
# rune-managed artifact (skill/agent/rule), and surfaces drift via
# additionalContext if the file has diverged from the registry.
#
# Embedded into the rune binary via include_str! in src/setup.rs.
# Lint gate in CI: shellcheck -s bash resources/hook.sh.
set -e

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# Only act on .claude/skills/, .claude/agents/, or .claude/rules/ .md files
MATCH=0
if [[ "$FILE_PATH" == *".claude/skills/"* ]] && [[ "$FILE_PATH" == *.md ]]; then
    MATCH=1
elif [[ "$FILE_PATH" == *".claude/agents/"* ]] && [[ "$FILE_PATH" == *.md ]]; then
    MATCH=1
elif [[ "$FILE_PATH" == *".claude/rules/"* ]] && [[ "$FILE_PATH" == *.md ]]; then
    MATCH=1
fi

if [[ "$MATCH" -eq 0 ]]; then
    exit 0
fi

# Find project root (walk up to find .claude/rune.toml)
DIR=$(dirname "$FILE_PATH")
while [[ "$DIR" != "/" ]]; do
    if [[ -f "$DIR/.claude/rune.toml" ]] || [[ -f "$DIR/rune.toml" ]]; then
        break
    fi
    DIR=$(dirname "$DIR")
done

if [[ ! -f "$DIR/.claude/rune.toml" ]]; then
    exit 0
fi

# Run rune check on the specific file
OUTPUT=$(rune check --file "$FILE_PATH" --project "$DIR" 2>&1) || true

if [[ -n "$OUTPUT" ]] && [[ "$OUTPUT" == *"DRIFTED"* ]]; then
    # Surface to Claude via additionalContext
    ESCAPED=$(echo "$OUTPUT" | jq -Rs .)
    printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":%s}}' "$ESCAPED"
fi

exit 0
