#!/usr/bin/env bash
# PostToolUseFailure hook: record a check command that failed, as evidence for
# the learning-loop Stop checklist.
#
# Claude Code fires PostToolUseFailure, not PostToolUse, when a Bash command
# exits non-zero, and documents the first line of `error` as `Exit code N`.
# That line is the only failure taken as a check outcome: a timeout, an
# interrupt (`is_interrupt`) or a shell that never started says nothing about
# the code under test, and neither does an exit status that reports the launch
# (126, 127, or flock's own codes: check_failure_counts). Which commands
# count, and why a pipe or a chain never does, is check_kind in lib/common.sh.
#
# The record is evidence, never a reward. Stop names a command that failed and
# then passed, or that still fails, next to the exact memories the session
# recalled; the model decides whether one of them caused it. Nothing here
# reaches the store: the hooks never open it.
#
# Every path prints `{}` and exits 0: a failed tool call must never be made
# worse by its hook.
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
payload="$(read_stdin_payload)"
printf '%s' "$payload" | jq -e '.tool_name == "Bash" and .is_interrupt != true' >/dev/null 2>&1 \
  || done_here
read_exact session_id jq -j '.session_id // empty' <<<"$payload" 2>/dev/null || session_id=""
read_exact cwd jq -j '.cwd // empty' <<<"$payload" 2>/dev/null || cwd=""
[ -n "$session_id" ] || done_here
[ -n "$cwd" ] || cwd="$PWD"

resolve_config "$cwd" || done_here
learning_loop_enabled || done_here
read_exact error_text jq -j '.error // empty' <<<"$payload" 2>/dev/null || done_here
# shellcheck disable=SC2154 # read_exact sets error_text (printf -v)
read_exact exit_code failure_exit_code "$error_text" || done_here
read_exact check_command jq -j '.tool_input.command // empty' <<<"$payload" 2>/dev/null || done_here
# shellcheck disable=SC2154 # read_exact sets exit_code and check_command (printf -v)
check_failure_counts "$check_command" "$exit_code" || done_here
read_exact checks_dir record_dir_path "check-outcomes" "$session_id" || done_here
# shellcheck disable=SC2154 # read_exact sets checks_dir and check_command (printf -v)
record_check_outcome "$checks_dir" "$check_command" fail || true
done_here
