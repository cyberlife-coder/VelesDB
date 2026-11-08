#!/usr/bin/env bash
# Devin CLI PostCompaction hook: re-anchor the session's memory after the
# compactor ran.
#
# Devin has no pre-compaction event: nothing can nudge a save *before* the
# transcript is condensed, so the design inverts claude-code's pre-compact.sh
# (which blocks once to get the save in first). Here the moment that matters
# is right after — restore what a save preserved, and get durable what the
# summary could not hold. The hook emits additionalContext rather than
# blocking: compaction already happened, a refusal has nothing to stop.
#
# Every firing emits: a session can compact more than once, and each time the
# transcript just lost what the last load had restored. This is deliberately
# NOT deduplicated through first_context_emit.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)" # exact-read-ok: a line below sources lib/ from this value, so a byte lost here fails loudly instead of naming another tree
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

require_jq

read_exact payload read_stdin_payload
# shellcheck disable=SC2154 # read_exact sets payload (printf -v)
read_exact session_id jq -j '.session_id // empty' <<<"$payload" 2>/dev/null || session_id=""
read_exact cwd jq -j '.cwd // empty' <<<"$payload" 2>/dev/null || cwd=""
if [ -z "$cwd" ]; then
  cwd="${DEVIN_PROJECT_DIR:-$PWD}"
fi
if [ -z "$session_id" ]; then
  echo '{}'
  exit 0
fi

resolve_config "$cwd"
adopt_working_session "$session_id" any || true

context="Context was just compacted (velesdb-memory): call load_working_context(project=\"$PROJECT\", session=\"$SESSION\") again now — compaction discarded what an earlier load restored, and the saved distilled state (goal, constraints, decisions, pending actions) is the fastest way back. Then: if this session diagnosed incidents, root causes, measured facts or decisions that are not yet durable, remember + relate them NOW — the transcript detail that would let you write them accurately is exactly what compaction just discarded, and a fact written from the summary alone is worse than one written while the evidence was on screen. Do NOT score feedback on memories from the compacted half of the session: only outcomes observed after this point count."

jq -n --arg ctx "$context" \
  '{hookSpecificOutput: {hookEventName: "PostCompaction", additionalContext: $ctx}}'
