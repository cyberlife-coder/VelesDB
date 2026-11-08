#!/usr/bin/env bash
# Devin CLI PostToolUse hook: mark the session after a successful VelesDB
# recall, record the working context a save or load names, and record shell
# check outcomes as feedback evidence.
#
# Devin has one PostToolUse event where Claude Code has two (PostToolUse and
# PostToolUseFailure): `.tool_response` is `{success, output, error}`, so the
# same script sees a passed command and a failed one. A Devin exec result
# reports the command's exit status as a trailing `Exit code[: ]N` line in
# output; that line — not the success flag alone — is what a check outcome is
# recorded from, because a backgrounded or killed call can report success
# without the check ever having run to a verdict. Commands that never
# reached a status line record nothing, same contract as check_kind in
# lib/common.sh.
#
# Every path prints `{}` and exits 0: a PostToolUse hook must never make a
# tool call worse.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)" # exact-read-ok: a line below sources lib/ from this value, so a byte lost here fails loudly instead of naming another tree
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

done_here() {
  echo '{}'
  exit 0
}

command -v jq >/dev/null 2>&1 || done_here
read_exact payload read_stdin_payload
# shellcheck disable=SC2154 # read_exact sets payload (printf -v)
tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty' 2>/dev/null || true)" # exact-read-ok: a tool name is one word, never a line
read_exact session_id jq -j '.session_id // empty' <<<"$payload" 2>/dev/null || session_id=""
read_exact cwd jq -j '.cwd // empty' <<<"$payload" 2>/dev/null || cwd=""
[ -n "$cwd" ] || cwd="${DEVIN_PROJECT_DIR:-$PWD}"

resolve_config "$cwd" || done_here

# --- VelesDB recall evidence ------------------------------------------------
if [ -n "$session_id" ] && successful_memory_recall "$payload"; then
  pending_status=2
  if read_exact pending_dir record_dir_path "pending-recall" "$session_id"; then
    # shellcheck disable=SC2154 # read_exact sets pending_dir (printf -v)
    if promote_pending_recall \
      "$pending_dir" "recall" "$session_id" "$payload"; then
      pending_status=0
    else
      pending_status=$?
    fi
  fi
  if [ "$pending_status" -ne 2 ] \
    && learning_loop_enabled \
    && recall_targets_current_project "$payload"; then
    learning_marker_identity marker_id "$session_id" # exact-read-ok: printf -v from arguments this process already holds; nothing is read
    # shellcheck disable=SC2154 # learning_marker_identity sets marker_id (printf -v)
    if read_exact marker_path sentinel_path "recall" "$marker_id"; then
      touch_private_marker "$marker_path" || true
    fi
  fi
fi

# Credit assignment (lib/common.sh): the ids a successful recall returned, for
# the Stop checklist to name — only where the learning loop is opted in.
if [ -n "$session_id" ] && successful_memory_recall "$payload" \
  && learning_loop_enabled && recall_targets_current_project "$payload" \
  && read_exact recalled_dir record_dir_path "recalled-memories" "$session_id"; then
  # shellcheck disable=SC2154 # read_exact sets recalled_dir (printf -v)
  record_recalled_memories "$recalled_dir" "$tool_name" "$payload" || true
fi

# A session this conversation saves becomes the one the SessionStart and Stop
# reminders name; one it only loads, the one SessionStart asks it to load.
if [ -n "$session_id" ]; then
  remember_working_session "$session_id" "$tool_name" "$payload" || true
fi

# --- exec check outcomes ----------------------------------------------------
# timeout=0 backgrounds an exec call: its result reports the launch, never a
# verdict, so it can record nothing regardless of the success flag.
[ "$tool_name" = "exec" ] || done_here
learning_loop_enabled || done_here
[ -n "$session_id" ] || done_here
printf '%s' "$payload" | jq -e '(.tool_input.timeout // 1) != 0' >/dev/null 2>&1 || done_here
read_exact check_command jq -j '.tool_input.command // empty' <<<"$payload" 2>/dev/null || done_here
# shellcheck disable=SC2154 # read_exact sets check_command (printf -v)
[ -n "$check_command" ] || done_here

# The exit status Devin appends to a finished exec result. A result carrying
# none — a killed, timed-out or backgrounded call — is no check outcome.
# shellcheck disable=SC2016 # the grep pipeline runs inside sh -c; $1 is its positional argument
read_exact_line exit_code sh -c 'printf %s "$1" | grep -oE "Exit code:? [0-9]+" | tail -n 1 | grep -oE "[0-9]+$"' _ \
  "$(printf '%s' "$payload" | jq -j '[.tool_response.output // "", .tool_response.error // ""] | join("\n")')" \
  || exit_code=""; :
[ -n "$exit_code" ] || done_here
read_exact success jq -j '.tool_response.success // empty' <<<"$payload" 2>/dev/null || success=""

if [ "$exit_code" = "0" ]; then
  # A pass needs the host's verdict too: Exit code 0 in output a failed launch
  # echoed back is not one. record_check_outcome keeps only real checks.
  [ "$success" = "true" ] || done_here
  read_exact checks_dir record_dir_path "check-outcomes" "$session_id" || done_here
  # shellcheck disable=SC2154 # read_exact sets checks_dir (printf -v)
  record_check_outcome "$checks_dir" "$CONFIG_ROOT" "$check_command" pass || true
else
  check_failure_counts "$check_command" "$exit_code" >/dev/null 2>&1 || done_here
  read_exact checks_dir record_dir_path "check-outcomes" "$session_id" || done_here
  # shellcheck disable=SC2154 # read_exact sets checks_dir (printf -v)
  record_check_outcome "$checks_dir" "$CONFIG_ROOT" "$check_command" fail || true
fi
done_here
