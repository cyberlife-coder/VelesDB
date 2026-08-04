#!/usr/bin/env bash
# Refuse the first apply_patch in an opted-in project until a VelesDB recall
# has completed successfully in the same Codex session.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

if ! command -v jq >/dev/null 2>&1; then
  echo "VelesDB learning-loop guard: jq is unavailable, so repository policy and same-session recall cannot be verified. Install jq before retrying apply_patch." >&2
  exit 2
fi
payload="$(read_stdin_payload)"
tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty' 2>/dev/null || true)"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)"
session_id="$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)"
patch="$(printf '%s' "$payload" | jq -r '.tool_input.command // empty' 2>/dev/null || true)"

[ "$tool_name" = "apply_patch" ] || { echo '{}'; exit 0; }
[ -n "$cwd" ] || cwd="$PWD"
targets="$(printf '%s' "$patch" | awk '
  /^\*\*\* (Add|Update|Delete) File: / {
    sub(/^\*\*\* (Add|Update|Delete) File: /, "")
    print
  }
  /^\*\*\* Move to: / {
    sub(/^\*\*\* Move to: /, "")
    print
  }
')"
[ -n "$targets" ] || targets="."

needs_checkpoint="false"
dirty_projects='[]'
while IFS= read -r target_path; do
  [ -n "$target_path" ] || continue
  case "$target_path" in
    /*) target="$target_path" ;;
    *) target="$cwd/$target_path" ;;
  esac
  if [ "$target_path" = "." ]; then
    policy_start="$cwd"
  else
    policy_start="$(dirname "$target")"
  fi
  if ! resolve_config "$policy_start"; then
    echo "VelesDB learning-loop guard: a physical patch target could not be resolved safely; apply_patch remains refused." >&2
    exit 2
  fi
  if [ -L "$target" ]; then
    lexical_enforced="$ENFORCE_LEARNING_LOOP"
    if ! resolved_target="$(resolve_final_symlink "$target")" \
      || ! resolve_config "$(dirname "$resolved_target")"; then
      echo "VelesDB learning-loop guard: a final symlink target could not be resolved safely; recall cannot authorize this patch. Retry with a physical non-symlink path." >&2
      exit 2
    fi
    if [ "$lexical_enforced" = "true" ] || learning_loop_enabled; then
      echo "VelesDB learning-loop guard: a final symlink crosses an opted-in policy boundary; recall markers cannot safely authorize its patch semantics. Retry with the physical non-symlink path." >&2
      exit 2
    fi
    continue
  fi
  learning_loop_enabled || continue
  [ -n "$session_id" ] || {
    echo "VelesDB learning-loop guard: the hook payload has no session_id, so same-session recall cannot be verified. Retry in a valid Codex session before editing." >&2
    exit 2
  }

  marker_id="$(learning_marker_identity "$session_id")"
  if ! sentinel="$(sentinel_path "codex-recall" "$marker_id")"; then
    echo "VelesDB learning-loop guard: private hook-state storage is unsafe or unavailable; apply_patch remains refused." >&2
    exit 2
  fi
  if ! valid_private_marker "$sentinel"; then
    if [ -e "$sentinel" ] || [ -L "$sentinel" ]; then
      echo "VelesDB learning-loop guard: a recall marker is linked or malformed, so same-session recall cannot be verified. Repair the private hook-state directory before retrying apply_patch." >&2
      exit 2
    fi
    if ! pending_dir="$(record_dir_path "codex-pending-recall" "$session_id")"; then
      echo "VelesDB learning-loop guard: private hook-state storage is unsafe or unavailable; apply_patch remains refused." >&2
      exit 2
    fi
    if ! record_current_project "$pending_dir"; then
      echo "VelesDB learning-loop guard: could not persist the pending repository identity; apply_patch remains refused." >&2
      exit 2
    fi
    echo "VelesDB learning-loop guard: run a velesdb-memory recall_fused for every opted-in repository touched by this patch and wait for its successful result. A timeout or failed recall does not unlock apply_patch." >&2
    exit 2
  fi
  record="$(project_record)"
  dirty_projects="$(jq -cn \
    --argjson projects "$dirty_projects" \
    --argjson record "$record" \
    '($projects + [$record]) | unique_by(.root)')"
  needs_checkpoint="true"
done <<< "$targets"

# Mark only after every target has passed, so a rejected multi-repository patch
# does not create a false edit checkpoint.
if [ "$needs_checkpoint" = "true" ]; then
  if ! dirty_dir="$(record_dir_path "codex-learning-dirty" "$session_id")"; then
    echo "VelesDB learning-loop guard: private hook-state storage is unsafe or unavailable; apply_patch remains refused." >&2
    exit 2
  fi
  while IFS= read -r record; do
    if ! record_project_json "$dirty_dir" "$record"; then
      echo "VelesDB learning-loop guard: could not persist every edited repository identity; apply_patch remains refused." >&2
      exit 2
    fi
  done < <(printf '%s' "$dirty_projects" | jq -c '.[]')
fi
echo '{}'
