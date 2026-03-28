#!/bin/bash
set -euo pipefail
# Block edits to secret/credential files (Hard Rule 2)
INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# Match dotfiles: .env, .env.*, .secret
# Match extensions: *.pem, *.key, *.p12, *.pfx, *.keystore
# Match names: credentials.*
if echo "$FILE_PATH" | grep -qE '(^|/)\.(env|secret)($|\.)|credentials\.|\.pem$|\.key$|\.p12$|\.pfx$|\.keystore$'; then
  echo "Blocked: editing secret/credential files is not allowed: $FILE_PATH" >&2
  exit 2
fi
