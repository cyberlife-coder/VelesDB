#!/usr/bin/env bash
# Mark the session only after an opted-in VelesDB recall completed successfully,
# and record the working context a successful save or load names.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)" # exact-read-ok: the next line sources lib/ from this value, so a byte lost here fails loudly instead of naming another tree
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

require_jq
payload="$(read_stdin_payload)"
read_exact cwd jq -j '.cwd // empty' <<<"$payload" 2>/dev/null || cwd=""
read_exact session_id jq -j '.session_id // empty' <<<"$payload" 2>/dev/null || session_id=""
# The tool name is read once, here: the recall check and the working-context
# recording both take it, so neither parses the payload for it again.
tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty' 2>/dev/null || true)"
[ -n "$cwd" ] || cwd="$PWD"

resolve_config "$cwd"
if [ -n "$session_id" ] && successful_memory_recall "$tool_name" "$payload"; then
  pending_status=2
  if read_exact pending_dir record_dir_path "codex-pending-recall" "$session_id"; then
    # shellcheck disable=SC2154 # read_exact sets pending_dir (printf -v)
    if promote_pending_recall \
      "$pending_dir" "codex-recall" "$session_id" "$payload"; then
      pending_status=0
    else
      pending_status=$?
    fi
  fi
  # The recall also unlocks the root it ran from, whether or not it promoted a
  # pending edit elsewhere: a parent whose subagent's worktree waited must not
  # have its own next edit refused (#2308). Malformed or unreadable pending
  # state (2) marks nothing.
  if [ "$pending_status" -ne 2 ] \
    && learning_loop_enabled \
    && recall_targets_current_project "$payload"; then
    learning_marker_identity marker_id "$session_id"
    # shellcheck disable=SC2154 # learning_marker_identity sets marker_id (printf -v)
    if read_exact marker_path sentinel_path "codex-recall" "$marker_id"; then
      touch_private_marker "$marker_path" || true
    fi
  fi
fi

# A session this conversation saves becomes the one the SessionStart and Stop
# reminders name; one it only loads, the one SessionStart asks it to load
# (lib/common.sh). Like the recall check, it takes the tool name read above.
if [ -n "$session_id" ]; then
  remember_working_session "$session_id" "$tool_name" "$payload" || true
fi
echo '{}'
