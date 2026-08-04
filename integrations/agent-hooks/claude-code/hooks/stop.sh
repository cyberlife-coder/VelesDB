#!/usr/bin/env bash
# Stop hook: remind the model, once per session, to save its working context
# before finishing. Blocks the first Stop with decision:block + reason (the
# model reads `reason` and acts on it); every later Stop in the same session
# passes through untouched, guarded by a sentinel file keyed on session_id
# (Claude Code does not hand hooks a "have I already blocked once" flag, so
# the hook tracks it itself — see lib/common.sh sentinel_path).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

require_jq

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

sentinel="$(sentinel_path "stop" "$session_id")"

if [ -f "$sentinel" ]; then
  # Already reminded this session — let Claude stop normally.
  echo '{}'
  exit 0
fi

: > "$sentinel"

if learning_loop_enabled; then
  reason="Before finishing, complete the VelesDB learning loop: 1. Recall prior patterns; 2. Decision: remember each non-trivial decision; 3. Causality: relate each decision to its cause with an outgoing relation; 4. Feedback: send feedback for every recalled memory that helped or misled. Then call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") with the distilled state and stop."
else
  reason="Before finishing: call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") via velesdb-memory with the distilled state (goal, key decisions, verified facts, pending actions), then stop."
fi

jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
