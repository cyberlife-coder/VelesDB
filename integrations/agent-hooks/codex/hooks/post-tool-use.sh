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
if [ -n "$session_id" ] && successful_memory_recall "$payload"; then
  pending_status=2
  if pending_dir="$(record_dir_path "codex-pending-recall" "$session_id")"; then
    if promote_pending_recall \
      "$pending_dir" "codex-recall" "$session_id" "$payload"; then
      pending_status=0
    else
      pending_status=$?
    fi
  fi
  if [ "$pending_status" -ne 0 ]; then
    if { [ "$pending_status" -eq 1 ] || [ "$pending_status" -eq 3 ]; } \
      && learning_loop_enabled \
      && recall_targets_current_project "$payload"; then
      marker_id="$(learning_marker_identity "$session_id")"
      if marker_path="$(sentinel_path "codex-recall" "$marker_id")"; then
        touch_private_marker "$marker_path" || true
      fi
    fi
  fi
fi

echo '{}'
