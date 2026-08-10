#!/usr/bin/env bash
# Refuse the first repository edit in an opted-in project until a VelesDB
# recall has completed successfully in the same agent session.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

if ! command -v jq >/dev/null 2>&1; then
  echo "VelesDB learning-loop guard: jq is unavailable, so repository policy and same-session recall cannot be verified. Install jq before retrying this Edit/Write." >&2
  exit 2
fi
payload="$(read_stdin_payload)"
tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty' 2>/dev/null || true)"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)"
session_id="$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)"
target_path="$(printf '%s' "$payload" | jq -r '.tool_input.file_path // empty' 2>/dev/null || true)"

case "$tool_name" in
  Edit|Write) ;;
  *) echo '{}'; exit 0 ;;
esac

[ -n "$cwd" ] || cwd="$PWD"
if [ -n "$target_path" ]; then
  case "$target_path" in
    /*) target="$target_path" ;;
    *) target="$cwd/$target_path" ;;
  esac
  policy_start="$(dirname "$target")"
else
  # A covered tool without its documented target is evaluated conservatively
  # from cwd rather than treated as automatically unconfigured.
  policy_start="$cwd"
fi
if ! resolve_config "$policy_start"; then
  echo "VelesDB learning-loop guard: the physical edit target could not be resolved safely; the edit remains refused." >&2
  exit 2
fi
if [ -L "${target:-}" ]; then
  lexical_enforced="$ENFORCE_LEARNING_LOOP"
  if ! resolved_target="$(resolve_final_symlink "$target")" \
    || ! resolve_config "$(dirname "$resolved_target")"; then
    echo "VelesDB learning-loop guard: a final symlink target could not be resolved safely; recall cannot authorize this edit. Retry with a physical non-symlink path." >&2
    exit 2
  fi
  if [ "$lexical_enforced" = "true" ] || learning_loop_enabled; then
    echo "VelesDB learning-loop guard: a final symlink crosses an opted-in policy boundary; recall markers cannot safely authorize its edit semantics. Retry with the physical non-symlink path." >&2
    exit 2
  fi
  echo '{}'
  exit 0
fi
learning_loop_enabled || { echo '{}'; exit 0; }
[ -n "$session_id" ] || {
  echo "VelesDB learning-loop guard: the hook payload has no session_id, so same-session recall cannot be verified. Retry in a valid agent session before editing." >&2
  exit 2
}

marker_id="$(learning_marker_identity "$session_id")"
if ! sentinel="$(sentinel_path "recall" "$marker_id")"; then
  echo "VelesDB learning-loop guard: private hook-state storage is unsafe or unavailable; the edit remains refused." >&2
  exit 2
fi
if valid_private_marker "$sentinel"; then
  # Stop consumes this marker after the covered edit batch and can therefore
  # remind again after a later edit without looping on the continuation.
  if ! dirty_dir="$(record_dir_path "learning-dirty" "$session_id")"; then
    echo "VelesDB learning-loop guard: private hook-state storage is unsafe or unavailable; the edit remains refused." >&2
    exit 2
  fi
  if ! record_current_project "$dirty_dir"; then
    echo "VelesDB learning-loop guard: could not persist the edited repository identity; the edit remains refused." >&2
    exit 2
  fi
  echo '{}'
  exit 0
fi
if [ -e "$sentinel" ] || [ -L "$sentinel" ]; then
  echo "VelesDB learning-loop guard: the recall marker is linked or malformed, so same-session recall cannot be verified. Repair the private hook-state directory before retrying this Edit/Write." >&2
  exit 2
fi

if ! pending_dir="$(record_dir_path "pending-recall" "$session_id")"; then
  echo "VelesDB learning-loop guard: private hook-state storage is unsafe or unavailable; the edit remains refused." >&2
  exit 2
fi
if ! record_current_project "$pending_dir"; then
  echo "VelesDB learning-loop guard: could not persist the pending repository identity; the edit remains refused." >&2
  exit 2
fi
echo "VelesDB learning-loop guard: run a velesdb-memory recall_fused for this code area and wait for its successful result before the first Edit/Write. A timeout or failed recall does not unlock the edit." >&2
exit 2
