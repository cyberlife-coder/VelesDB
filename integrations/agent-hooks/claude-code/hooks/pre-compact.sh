#!/usr/bin/env bash
# PreCompact hook: remind the model to save its working context before
# transcript compaction discards detail.
#
# IMPORTANT deviation from a naive "additionalContext" design: PreCompact's
# output schema does NOT support hookSpecificOutput/additionalContext (only
# SessionStart and Stop do) — the only channel that reaches the model is
# `decision:"block"` + `reason`, which also blocks the compaction attempt.
# So this hook blocks the FIRST PreCompact per session (the model reads
# `reason`, saves, and Claude Code will naturally re-attempt compaction),
# then passes every later PreCompact through untouched via the same
# sentinel-file pattern as stop.sh. Blocking every single PreCompact would
# be unsafe (auto-compaction can fire repeatedly as a long session grows;
# refusing it every time risks the transcript never compacting).
#
# TODO(V2b): once compile_transcript ships, this hook can compile the
# transcript directly instead of only nudging the model to save by hand.
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

sentinel="$(sentinel_path "precompact" "$session_id")"

if [ -f "$sentinel" ]; then
  echo '{}'
  exit 0
fi

: > "$sentinel"

reason="Before compaction: call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") via velesdb-memory with the distilled state so nothing is lost, then retry — compaction will proceed on the next attempt."

jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
