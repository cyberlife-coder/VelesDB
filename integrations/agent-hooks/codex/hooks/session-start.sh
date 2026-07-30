#!/usr/bin/env bash
# Codex CLI SessionStart hook: tell the model to resume its rolling working
# context, and — when the session restarts *because of a compaction* — to
# compile what is about to be lost.
#
# Why SessionStart carries the compaction reminder here, unlike the Claude
# Code integration which has a dedicated pre-compact.sh: per the Codex hooks
# reference (checked 2026-07-25), `PreCompact` and `PostCompact` do NOT
# support `hookSpecificOutput.additionalContext`, and no `decision`/`reason`
# output is documented for them either — so a Codex PreCompact hook has no
# documented channel that reaches the model at all. `SessionStart` does, and
# it fires with `source: "compact"` after a compaction, so that is the only
# documented place the compaction advice can actually be delivered. It lands
# after the fact rather than before it; that is a real downgrade, spelled out
# in ../README.md.
#
# Why a hook and not a second velesdb-memory process: the store is
# mono-process (flock). The MCP server already running inside this Codex
# session holds the lock, so this script cannot open the store itself — it can
# only steer the model to call the *session's own* MCP tool. See
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

# Documented values: startup | resume | clear | compact.
source_kind="$(printf '%s' "$payload" | jq -r '.source // empty')"

resolve_config "$cwd"

context="Session memory (velesdb-memory): call load_working_context(project=\"$PROJECT\", session=\"$SESSION\") as your first action, unless you already loaded it earlier this session. It restores the prior distilled state (goal, constraints, verified facts, decisions, pending actions) left by save_working_context, so work continues instead of re-deriving context from scratch. load_working_context returns {found, working, other_sessions}: read 'working' for the state. If 'found' is false, nothing was saved under that EXACT project+session — but check 'other_sessions' before starting fresh: a similarly-named session listed there means the session id was a typo, not a new task. 'other_sessions' is filled in on a hit too, so if one of them looks more like the session you meant, you may have just resumed the wrong work. Reinforcement loop: whenever a memory surfaced by recall/recall_fused (or pulled into compile_context — its decision carries the memory_id) actually helps you, call feedback(id, true) with the id_str string; if it misled you, feedback(id, false). This is what makes ranking improve with use — skipping it keeps confidence flat."

if [ "$source_kind" = "compact" ]; then
  context="$context This session restarted after a COMPACTION, so detail from the previous transcript was just discarded lossily. Codex hooks cannot intercept compaction before it happens, so this is the first moment anything can be said about it: call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") now with whatever distilled state survived, and from here on save it again whenever the working state changes meaningfully rather than waiting for the end of the session."
fi

jq -n --arg ctx "$context" \
  '{hookSpecificOutput: {hookEventName: "SessionStart", additionalContext: $ctx}}'
