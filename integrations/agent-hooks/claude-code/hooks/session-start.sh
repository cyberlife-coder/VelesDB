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

require_jq

payload="$(read_stdin_payload)"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
if [ -z "$cwd" ]; then
  cwd="$PWD"
fi

resolve_config "$cwd"

context="Session memory (velesdb-memory): call load_working_context(project=\"$PROJECT\", session=\"$SESSION\") as your first action, unless you already loaded it earlier this session. It restores the prior distilled state (goal, constraints, verified facts, decisions, pending actions) left by save_working_context, so work continues instead of re-deriving context from scratch. If it returns null, nothing was saved yet — proceed normally."

jq -n --arg ctx "$context" \
  '{hookSpecificOutput: {hookEventName: "SessionStart", additionalContext: $ctx}}'
