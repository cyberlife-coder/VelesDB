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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)" # exact-read-ok: a line below sources lib/ from this value, so a byte lost here fails loudly instead of naming another tree
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

require_jq

payload="$(read_stdin_payload)"
read_exact session_id jq -j '.session_id // empty' <<<"$payload" 2>/dev/null || session_id=""
read_exact cwd jq -j '.cwd // empty' <<<"$payload" 2>/dev/null || cwd=""
read_exact transcript_path jq -j '.transcript_path // empty' <<<"$payload" 2>/dev/null || transcript_path=""

if [ -z "$cwd" ]; then
  cwd="$PWD"
fi
if [ -z "$session_id" ]; then
  session_id="unknown-session"
fi

resolve_config "$cwd"
adopt_working_session "$session_id" save || true

if ! read_exact sentinel sentinel_path "precompact" "$session_id"; then
  reason="VelesDB private hook-state storage is unsafe or unavailable. Keep the session open, repair the per-user state directory, and retry compaction."
  jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
  exit 0
fi

# shellcheck disable=SC2154 # read_exact sets sentinel (printf -v)
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

# compile_transcript(path=…) resolves only under the server's
# VELESDB_MEMORY_INGEST_ROOTS allowlist, and a harness transcript lives in
# the harness's own state directory (~/.claude/projects) — normally outside
# every configured root. Nudging the model to pass transcript_path verbatim
# mandates a call the allowlist refuses (#2357), so the reason names the
# path AND the constraint, plus the two routes that actually work.
if [ -n "$transcript_path" ]; then
  transcript_clause="the transcript about to be compacted is at $transcript_path — pass it to compile_transcript (velesdb-memory) as 'path' with a query and a token_budget to deterministically compress it (duplicates dropped, logs collapsed, code/negative-constraints preserved verbatim) instead of losing detail to lossy compaction. compile_transcript(path=…) only reads files under the server's VELESDB_MEMORY_INGEST_ROOTS allowlist — a harness state-directory transcript like this one is normally outside it, so if the call answers 'ingestion is disabled' or 'outside the configured ingest roots', copy the file inside an allowed root (e.g. under your cwd) and pass that path, or pass the transcript text inline via 'transcript'."
else
  transcript_clause="the transcript about to be compacted is exactly what compile_transcript exists for — call it (velesdb-memory) with a query and a token_budget to deterministically compress it (duplicates dropped, logs collapsed, code/negative-constraints preserved verbatim) instead of losing detail to lossy compaction."
fi

reason="Before compaction: $transcript_clause Also call save_working_context(project=\"$PROJECT\", session=\"$SESSION\") with the distilled state (goal, decisions, pending actions) so nothing is lost even for content compile_transcript can't recover. Then retry — compaction will proceed on the next attempt."

jq -n --arg reason "$reason" '{decision: "block", reason: $reason}'
