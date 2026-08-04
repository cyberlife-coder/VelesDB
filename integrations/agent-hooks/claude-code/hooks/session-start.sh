#!/usr/bin/env bash
# SessionStart hook: tell the model to resume its rolling working context.
#
# Why a hook and not a second velesdb-memory process: the store is
# mono-process (flock). The MCP server already running inside this Claude
# Code session holds the lock, so this script cannot open the store itself —
# it can only steer the model to call the *session's own* MCP tool. See
# integrations/agent-hooks/README.md for the full constraint writeup.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/freshness.sh
source "$SCRIPT_DIR/lib/freshness.sh"

require_jq

payload="$(read_stdin_payload)"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)"
if [ -z "$cwd" ]; then
  cwd="$PWD"
fi

resolve_config "$cwd"

context="Session memory (velesdb-memory): call load_working_context(project=\"$PROJECT\", session=\"$SESSION\") as your first action, unless you already loaded it earlier this session. It restores the prior distilled state (goal, constraints, verified facts, decisions, pending actions) left by save_working_context, so work continues instead of re-deriving context from scratch. load_working_context returns {found, working, other_sessions}: read 'working' for the state. If 'found' is false, nothing was saved under that EXACT project+session — but check 'other_sessions' before starting fresh: a similarly-named session listed there means the session id was a typo, not a new task. 'other_sessions' is filled in on a hit too, so if one of them looks more like the session you meant, you may have just resumed the wrong work. Reinforcement loop: whenever a memory surfaced by recall/recall_fused (or pulled into compile_context — its decision carries the memory_id) actually helps you, call feedback(id, true) with the id_str string; if it misled you, feedback(id, false). This is what makes ranking improve with use — skipping it keeps confidence flat."

# A daemon behind the published release is worth one line, and only when it
# is true: an up-to-date session sees nothing. Best-effort by construction —
# `|| true` because no version check may ever cost the user a session start.
freshness="$(veles_freshness_notice 2>/dev/null || true)"
if [ -n "${freshness:-}" ]; then
  context="$context

$freshness"
fi

jq -n --arg ctx "$context" \
  '{hookSpecificOutput: {hookEventName: "SessionStart", additionalContext: $ctx}}'
