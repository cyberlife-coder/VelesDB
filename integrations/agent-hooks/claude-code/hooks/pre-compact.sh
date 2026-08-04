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
# V2b (compile_transcript shipped): the hook still cannot touch the store
# itself (mono-process flock constraint above), so it cannot compile the
# transcript directly either — it nudges the model to call compile_transcript
# itself, same mechanism as the save_working_context nudge below.
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

if ! sentinel="$(sentinel_path "precompact" "$session_id")"; then
  reason="VelesDB private hook-state storage is unsafe or unavailable. Keep the session open, repair the per-user state directory, and retry compaction."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi

if valid_private_marker "$sentinel"; then
  echo '{}'
  exit 0
fi
if [ -e "$sentinel" ] || [ -L "$sentinel" ]; then
  reason="VelesDB PreCompact marker is linked or malformed. Keep the session open, repair the private hook-state directory, and retry compaction."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi

if ! touch_private_marker "$sentinel"; then
  reason="VelesDB could not persist the PreCompact continuation marker. Keep the session open, inspect the private hook-state directory, and retry compaction."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi

reason="Before compaction: the transcript about to be compacted is exactly what compile_transcript exists for — call it (velesdb-memory) with a token_budget to deterministically compress it (duplicates dropped, logs collapsed, code/negative-constraints preserved verbatim) instead of losing detail to lossy compaction. Also call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") with the distilled state (goal, decisions, pending actions) so nothing is lost even for content compile_transcript can't recover. Then retry — compaction will proceed on the next attempt."

jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
