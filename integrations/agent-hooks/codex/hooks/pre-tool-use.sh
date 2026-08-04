#!/usr/bin/env bash
# Refuse the first apply_patch in an opted-in project until a VelesDB recall
# has completed successfully in the same Codex session.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

require_jq
payload="$(read_stdin_payload)"
tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty' 2>/dev/null || true)"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)"
session_id="$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)"

[ "$tool_name" = "apply_patch" ] || { echo '{}'; exit 0; }
[ -n "$cwd" ] || cwd="$PWD"
resolve_config "$cwd"
learning_loop_enabled || { echo '{}'; exit 0; }
[ -n "$session_id" ] || {
  echo "VelesDB learning-loop guard: the hook payload has no session_id, so same-session recall cannot be verified. Retry in a valid Codex session before editing." >&2
  exit 2
}

sentinel="$(sentinel_path "codex-recall" "$session_id")"
[ -f "$sentinel" ] && { echo '{}'; exit 0; }

echo "VelesDB learning-loop guard: run a velesdb-memory recall_fused for this code area and wait for its successful result before the first apply_patch. A timeout or failed recall does not unlock the edit." >&2
exit 2
