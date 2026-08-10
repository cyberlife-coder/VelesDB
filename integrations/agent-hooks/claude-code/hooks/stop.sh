#!/usr/bin/env bash
# Stop hook: continue once with the learning-loop checklist on the first Stop
# of an opted-in session and again after every later covered edit batch. The
# edit marker is consumed before the continuation, so its next Stop passes.
# Repositories without enforcement retain the older once-per-session save
# reminder.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

if ! command -v jq >/dev/null 2>&1; then
  printf '%s\n' '{"decision":"block","reason":"VelesDB learning-loop checkpoint cannot be verified because jq is unavailable. Restore jq on PATH, then retry Stop; do not finish while an edit queue may be pending."}'
  exit 0
fi

payload="$(read_stdin_payload)"
session_id="$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)"

if [ -z "$cwd" ]; then
  cwd="$PWD"
fi
if [ -z "$session_id" ]; then
  session_id="unknown-session"
fi

resolve_config "$cwd"

if ! dirty_dir="$(record_dir_path "learning-dirty" "$session_id")" \
  || ! generic_sentinel="$(sentinel_path "stop" "$session_id")" \
  || ! checkpoint_manifest="$(sentinel_path "learning-checkpoint-manifest" "$session_id")"; then
  reason="VelesDB private hook-state storage is unsafe or unavailable. Keep the session open, repair the per-user state directory, and retry Stop."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi

# A pending manifest means the previous Stop had captured every repository but
# did not durably acknowledge emitting its checklist (for example, the hook
# was interrupted between record cleanup and process exit). Re-emit the exact
# batch before doing anything else. A delivered manifest is the next Stop's
# acknowledgement and can be consumed before checking for a newer edit batch.
if valid_private_marker "$checkpoint_manifest"; then
  if ! jq -e '
    type == "object"
    and ((keys | sort) == ["state", "targets"])
    and (.state == "pending" or .state == "delivered")
    and ((.targets | type) == "array" and (.targets | length) > 0)
    and all(.targets[];
      type == "object"
      and ((keys | sort) == ["project", "root", "session"])
      and ((.project | type) == "string")
      and ((.session | type) == "string")
      and ((.root | type) == "string" and (.root | length) > 0))
  ' "$checkpoint_manifest" >/dev/null 2>&1; then
    reason="VelesDB checkpoint manifest is malformed. Keep the session open, inspect $checkpoint_manifest, and retry Stop; no captured repository identity was discarded."
    jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
    exit 0
  fi
  manifest_state="$(jq -r '.state' "$checkpoint_manifest")"
  if [ "$manifest_state" = "pending" ]; then
    targets="$(jq -c '.targets' "$checkpoint_manifest")"
    reason="Before finishing, complete the VelesDB learning loop for every edited repository in this recovered batch ($targets): 1. Recall prior patterns; 2. Decision: remember each non-trivial decision; 3. Causality: relate each decision to its cause and each incident to its root cause with outgoing relations; 4. Feedback: send feedback for every recalled memory that helped or misled. Then call save_working_context for every listed project/session with its distilled state and stop."
    response="$(jq -n --arg reason "$reason" '{decision: "block", reason: $reason}')"
    delivered_manifest="$(jq -c '.state = "delivered"' "$checkpoint_manifest")"
    printf '%s\n' "$response"
    write_private_marker "$checkpoint_manifest" "$delivered_manifest" || true
    exit 0
  fi
  if ! rm -f "$checkpoint_manifest"; then
    reason="VelesDB could not acknowledge a delivered checkpoint manifest. Keep the session open and retry Stop; the captured identities remain at $checkpoint_manifest."
    jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
    exit 0
  fi
elif [ -e "$checkpoint_manifest" ] || [ -L "$checkpoint_manifest" ]; then
  reason="VelesDB checkpoint manifest is linked or malformed. Keep the session open, inspect $checkpoint_manifest, and retry Stop."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi

# Edit targets can live outside the host cwd. The dirty queue is therefore
# session-wide and carries each resolved repository identity into Stop.
dirty_records=()
dirty_invalid="false"
if [ -L "$dirty_dir" ]; then
  dirty_invalid="true"
elif [ -d "$dirty_dir" ]; then
  for record_file in "$dirty_dir"/*.json; do
    [ -e "$record_file" ] || [ -L "$record_file" ] || continue
    if ! [ -f "$record_file" ] || [ -L "$record_file" ] \
      || ! valid_project_record "$record_file"; then
      dirty_invalid="true"
      break
    fi
    canonical="$(jq -c '{project, session, root}' "$record_file")"
    if [ "$(basename "$record_file")" != "$(safe_marker_key "$canonical").json" ]; then
      dirty_invalid="true"
      break
    fi
    dirty_records+=("$record_file")
  done
elif [ -e "$dirty_dir" ]; then
  dirty_invalid="true"
fi
if [ "$dirty_invalid" = "true" ]; then
  reason="VelesDB learning-loop checkpoint metadata is malformed or linked. Keep the session open, inspect $dirty_dir, restore valid private records, and retry Stop; no pending repository identity was discarded."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi
if [ "${#dirty_records[@]}" -gt 0 ]; then
  targets="$(jq -sc '[.[] | {project, session, root}]' "${dirty_records[@]}")"
  for record_file in "${dirty_records[@]}"; do
    root="$(jq -r '.root' "$record_file")"
    marker_id="$(printf '%s\n%s' "$session_id" "$root")"
    if ! checkpoint_marker="$(sentinel_path "stop" "$marker_id")" \
      || ! touch_private_marker "$checkpoint_marker"; then
      reason="VelesDB could not persist a repository checkpoint marker. Keep the session open and retry Stop; the edit queue remains intact at $dirty_dir."
      jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
      exit 0
    fi
  done
  if ! touch_private_marker "$generic_sentinel"; then
    reason="VelesDB could not persist the continuation marker. Keep the session open and retry Stop; the edit queue remains intact at $dirty_dir."
    jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
    exit 0
  fi
  pending_manifest="$(jq -cn --argjson targets "$targets" \
    '{state: "pending", targets: $targets}')"
  if ! write_private_marker "$checkpoint_manifest" "$pending_manifest"; then
    reason="VelesDB could not persist the complete checkpoint manifest. Keep the session open and retry Stop; the edit queue remains intact at $dirty_dir."
    jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
    exit 0
  fi
  reason="Before finishing, complete the VelesDB learning loop for every edited repository in this batch ($targets): 1. Recall prior patterns; 2. Decision: remember each non-trivial decision; 3. Causality: relate each decision to its cause and each incident to its root cause with outgoing relations; 4. Feedback: send feedback for every recalled memory that helped or misled. Then call save_working_context for every listed project/session with its distilled state and stop."
  failure_reason="$reason Storage warning: at least one checkpoint record could not be consumed; keep the session open after completing the checklist, inspect $dirty_dir, and retry Stop."
  if ! response="$(jq -n --arg reason "$reason" '{decision: "block", reason: $reason}')" \
    || ! failure_response="$(jq -n --arg reason "$failure_reason" '{decision: "block", reason: $reason}')"; then
    reason="VelesDB could not encode the repository checklist. Keep the session open and retry Stop; the edit records remain at $dirty_dir."
    jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
    exit 0
  fi
  for record_file in "${dirty_records[@]}"; do
    if ! rm -f "$record_file"; then
      printf '%s\n' "$failure_response"
      exit 0
    fi
  done
  rmdir "$dirty_dir" 2>/dev/null || true
  delivered_manifest="$(printf '%s' "$pending_manifest" | jq -c '.state = "delivered"')"
  printf '%s\n' "$response"
  write_private_marker "$checkpoint_manifest" "$delivered_manifest" || true
  exit 0
fi

if learning_loop_enabled; then
  marker_id="$(learning_marker_identity "$session_id")"
  if ! sentinel="$(sentinel_path "stop" "$marker_id")"; then
    reason="VelesDB private hook-state storage is unsafe or unavailable. Keep the session open, repair the per-user state directory, and retry Stop."
    jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
    exit 0
  fi
  if valid_private_marker "$sentinel"; then
    echo '{}'
    exit 0
  fi
  if [ -e "$sentinel" ] || [ -L "$sentinel" ]; then
    reason="VelesDB repository Stop marker is linked or malformed. Keep the session open, repair the private hook-state directory, and retry Stop."
    jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
    exit 0
  fi
  if ! touch_private_marker "$sentinel" \
    || ! touch_private_marker "$generic_sentinel"; then
    reason="VelesDB could not persist the Stop continuation marker. Keep the session open, inspect the private hook-state directory, and retry Stop."
    jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
    exit 0
  fi
  # A continuation may resume outside this repository; suppress the generic
  # reminder there without granting another opted-in repository's sentinel.
  reason="Before finishing, complete the VelesDB learning loop: 1. Recall prior patterns; 2. Decision: remember each non-trivial decision; 3. Causality: relate each decision to its cause and each incident to its root cause with outgoing relations; 4. Feedback: send feedback for every recalled memory that helped or misled. Then call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") with the distilled state and stop."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi

if valid_private_marker "$generic_sentinel"; then
  # Already reminded this session — let Claude stop normally.
  echo '{}'
  exit 0
fi
if [ -e "$generic_sentinel" ] || [ -L "$generic_sentinel" ]; then
  reason="VelesDB Stop marker is linked or malformed. Keep the session open, repair the private hook-state directory, and retry Stop."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi

if ! touch_private_marker "$generic_sentinel"; then
  reason="VelesDB could not persist the Stop continuation marker. Keep the session open, inspect the private hook-state directory, and retry Stop."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi
reason="Before finishing: call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") via velesdb-memory with the distilled state (goal, key decisions, verified facts, pending actions), then stop."

jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
