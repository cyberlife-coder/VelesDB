#!/usr/bin/env bash
# Mark the session only after an opted-in VelesDB recall completed successfully.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

require_jq
payload="$(read_stdin_payload)"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)"
session_id="$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)"
[ -n "$cwd" ] || cwd="$PWD"

resolve_config "$cwd"
if learning_loop_enabled && [ -n "$session_id" ] && successful_memory_recall "$payload"; then
  : > "$(sentinel_path "codex-recall" "$session_id")"
fi

echo '{}'
