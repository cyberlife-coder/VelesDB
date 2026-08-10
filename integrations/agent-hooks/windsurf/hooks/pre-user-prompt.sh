#!/usr/bin/env bash
# Windsurf pre_user_prompt hook: on the first prompt of a session, remind the
# model to load its working context from velesdb-memory.
#
# pre_user_prompt is the only Windsurf event wired here. Windsurf documents
# eleven others (pre/post read_code, write_code, run_command, mcp_tool_use,
# post_cascade_response[_with_transcript], post_setup_worktree), but none is
# a Stop/PreCompact equivalent and none is implemented yet — see
# ../../README.md#parity-across-harnesses. So this single reminder folds in
# BOTH halves of the loop the Claude Code hooks split across three events:
# load working context now, and save it again before the session ends.
#
# Caveat: per Windsurf's documented contract, a hook's stdout is shown in the
# Cascade UI (show_output), and the agent itself only sees stderr from a
# pre-hook that exits 2 — which would also block the user's prompt. So this
# reminder is addressed to the user, not injected into the model context.
#
# Windsurf sends a JSON payload on stdin with fields like trajectory_id,
# execution_id, timestamp, model_name, tool_info. There is no stable
# cross-request session id in that payload other than trajectory_id, which
# we use as the once-per-session sentinel key (falling back to the parent
# PID if it's ever absent).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

require_jq

payload="$(read_stdin_payload)"
session_id="$(printf '%s' "$payload" | jq -r '.trajectory_id // empty')"
if [ -z "$session_id" ]; then
  session_id="windsurf-${PPID:-$$}"
fi
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
if [ -z "$cwd" ]; then
  cwd="$PWD"
fi

resolve_config "$cwd"

if ! sentinel="$(sentinel_path "windsurf-prompt" "$session_id")"; then
  echo "[velesdb-memory] Private hook-state storage is unsafe or unavailable; the session-memory reminder could not be persisted. Repair the per-user state directory before relying on once-per-session reminders."
  exit 0
fi

if valid_private_marker "$sentinel"; then
  # Already reminded this session — pass through silently.
  exit 0
fi
if [ -e "$sentinel" ] || [ -L "$sentinel" ]; then
  echo "[velesdb-memory] The session reminder marker is linked or malformed; repair the private hook-state directory before relying on once-per-session reminders."
  exit 0
fi
if ! touch_private_marker "$sentinel"; then
  echo "[velesdb-memory] The session reminder marker could not be persisted; repair the private hook-state directory before relying on once-per-session reminders."
  exit 0
fi

cat <<EOF
[velesdb-memory] Session memory: call load_working_context(project="$PROJECT", session="$SESSION") as your first action, unless you already loaded it earlier this session. It restores the prior distilled state (goal, constraints, verified facts, decisions, pending actions) left by save_working_context, so work continues instead of re-deriving context from scratch. load_working_context returns {found, working, other_sessions}: read 'working' for the state. If 'found' is false, nothing was saved under that EXACT project+session — but check 'other_sessions' before starting fresh: a similarly-named session listed there means the session id was a typo, not a new task. 'other_sessions' is filled in on a hit too, so if one of them looks more like the session you meant, you may have just resumed the wrong work. Before finishing this session, call save_working_context(project="$PROJECT", session="$SESSION") with the distilled state — Windsurf has no separate end-of-session hook, so this is the only reminder you'll get. Reinforcement loop: whenever a memory surfaced by recall/recall_fused actually helps you, call feedback(id, true) with the id_str value; if it misled you, feedback(id, false).
EOF
