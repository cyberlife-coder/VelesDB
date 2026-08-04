#!/usr/bin/env bash
# Codex CLI Stop hook: remind the model, once per session, to save its working
# context before finishing.
#
# Codex documents two output channels for Stop: `decision: "block"` + `reason`
# (a continuation prompt) and `hookSpecificOutput.additionalContext`. This hook
# uses the blocking form, matching ../../claude-code/hooks/stop.sh — a reminder
# that merely adds context to a turn the model has already decided to end is a
# reminder it can ignore.
#
# It blocks the FIRST Stop per session and lets every later one through,
# guarded by a sentinel file keyed on `session_id` (Codex, like Claude Code,
# hands hooks no "have I already blocked once" flag — `stop_hook_active` marks
# that a stop hook is running, not that this one already fired).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

require_jq

payload="$(read_stdin_payload)"
session_id="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"

if [ -z "$cwd" ]; then
  cwd="$PWD"
fi
if [ -z "$session_id" ]; then
  session_id="unknown-session"
fi

resolve_config "$cwd"

sentinel="$(sentinel_path "codex-stop" "$session_id")"

if [ -f "$sentinel" ]; then
  # Already reminded this session — let Codex stop normally.
  echo '{}'
  exit 0
fi

: > "$sentinel"

if learning_loop_enabled; then
  reason="Before finishing, complete the VelesDB learning loop: (1) recall prior patterns; (2) remember each non-trivial decision; (3) relate each decision to its cause with an outgoing relation; (4) send feedback for every recalled memory that helped or misled. Then call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") with the distilled state and stop."
else
  reason="Before finishing: call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") via velesdb-memory with the distilled state (goal, key decisions, verified facts, pending actions), then stop."
fi

jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
