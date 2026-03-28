#!/bin/bash
set -euo pipefail
# Block destructive git commands (safety net)
INPUT=$(cat)
COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

# Check for destructive git operations:
# - push --force / -f (anywhere in args)
# - reset --hard
# - clean -f (including -fd, -fx, etc.)
# - checkout/restore -- (discard working tree changes)
# - branch -D (force-delete)
if echo "$COMMAND" | grep -qE 'git\s+push\b.*(\s--force\b|\s-f\b)'; then
  echo "Blocked: destructive git push detected: $COMMAND" >&2
  exit 2
fi
if echo "$COMMAND" | grep -qE 'git\s+reset\b.*--hard'; then
  echo "Blocked: git reset --hard detected: $COMMAND" >&2
  exit 2
fi
if echo "$COMMAND" | grep -qE 'git\s+clean\b.*-[a-zA-Z]*f'; then
  echo "Blocked: git clean -f detected: $COMMAND" >&2
  exit 2
fi
if echo "$COMMAND" | grep -qE 'git\s+(checkout|restore)\b.*--\s'; then
  echo "Blocked: destructive checkout/restore detected: $COMMAND" >&2
  exit 2
fi
if echo "$COMMAND" | grep -qE 'git\s+branch\b.*\s-D\b'; then
  echo "Blocked: git branch -D detected: $COMMAND" >&2
  exit 2
fi
