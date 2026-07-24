#!/usr/bin/env bash
# Windsurf pre_user_prompt hook: on the first prompt of a session, remind the
# model to load its working context from velesdb-memory.
#
# Windsurf only exposes one lifecycle hook event (pre_user_prompt) — no
# separate Stop/PreCompact equivalent — so this single reminder folds in
# BOTH halves of the loop the Claude Code hooks split across three events:
# load working context now, and save it again before the session ends.
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

sentinel="$(sentinel_path "windsurf-prompt" "$session_id")"

if [ -f "$sentinel" ]; then
  # Already reminded this session — pass through silently.
  exit 0
fi
: > "$sentinel"

cat <<EOF
[velesdb-memory] Session memory: call load_working_context(project="$PROJECT", session="$SESSION") as your first action, unless you already loaded it earlier this session. It restores the prior distilled state (goal, constraints, verified facts, decisions, pending actions) left by save_working_context, so work continues instead of re-deriving context from scratch. If it returns null, nothing was saved yet — proceed normally. Before finishing this session, call save_working_context(project="$PROJECT", session="$SESSION") with the distilled state — Windsurf has no separate end-of-session hook, so this is the only reminder you'll get. Reinforcement loop: whenever a memory surfaced by recall/recall_fused actually helps you, call feedback(id, true) with the id_str value; if it misled you, feedback(id, false).
EOF
