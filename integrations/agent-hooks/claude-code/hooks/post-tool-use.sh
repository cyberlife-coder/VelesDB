#!/usr/bin/env bash
# PostToolUse hook: shrink an oversized tool result BEFORE it enters the
# agent's context, using the deterministic VelesDB context compiler.
#
# Why this hook is different from the other three. SessionStart, Stop and
# PreCompact can only *nudge the model* — they hand it a reason string and
# hope it calls the right tool. PostToolUse is the one event whose output
# schema can REPLACE what the model sees
# (`hookSpecificOutput.updatedToolOutput`), so it is the only place where
# VelesDB can reduce the payload of every subsequent API call rather than
# merely advising. A 300 KB `Bash` result compressed here is 300 KB that
# never enters the transcript, and therefore never gets re-sent on every
# later turn.
#
# It compiles in a SEPARATE PROCESS and never opens the store: the agent's
# own velesdb-memory MCP server already holds the store's single-writer
# `flock`, and this hook runs synchronously after every tool call. That is
# exactly what `velesdb-memory compile-stdin` exists for — the compiler
# (`ContextCompiler::compile`) is pure: no store, no index, no clock.
#
# Safety rules, in order of importance — a hook that runs on EVERY tool call
# must never lose data and never hang:
#   1. Nothing is deleted. The untouched original is written to a temp file
#      and its path is quoted in the replacement, so the agent can `Read` it
#      whenever the compiled view is not enough. This is the out-of-store
#      equivalent of a retrieval handle.
#   2. Identity fallback everywhere. Missing `jq`, missing or too-old binary,
#      compilation error, empty compiled output, unreadable JSON — every one
#      of them emits `{}` and leaves the tool result exactly as it was.
#   3. Bounded. The compile call runs under a watchdog (see
#      lib/common.sh run_with_watchdog); a binary predating `compile-stdin`
#      would otherwise treat our piped stdin as MCP traffic and hang.
#   4. Tool allowlist. Only tools whose output is prose/logs are compressed.
#      `Read` and `Edit` are deliberately NEVER in it: the model needs file
#      contents verbatim, byte for byte.
#   5. Fidelity. A compilation the compiler reports as `risk: high` is REFUSED,
#      not shipped: `high` means at least one fragment it classifies as
#      critical — a code fence, a negative constraint, an exact value, a URL —
#      did not survive verbatim. Rule 1 is not a substitute here. On this path
#      the compiler runs with no store and no bridge, so the `ctx://source/…`
#      handles it mints resolve to NOTHING; the temp file is the only way back,
#      and a model that was never told to look will not look.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=./lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"

# Identity fallback: emit an empty decision and leave the tool result alone.
passthrough() {
  echo '{}'
  exit 0
}

command -v jq >/dev/null 2>&1 || passthrough

payload="$(read_stdin_payload)"

# A malformed payload is not something to fail loudly on: it would mean
# every tool call in the session errors.
printf '%s' "$payload" | jq -e . >/dev/null 2>&1 || passthrough

tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty')"
session_id="$(printf '%s' "$payload" | jq -r '.session_id // "unknown-session"')"
tool_use_id="$(printf '%s' "$payload" | jq -r '.tool_use_id // "unknown-call"')"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
[ -n "$cwd" ] || cwd="$PWD"

# This happens before the compiler allowlist and size checks: MCP recall
# results are normally small and are evidence for the learning-loop guard,
# not candidates for context compression.
resolve_config "$cwd"
if learning_loop_enabled && successful_memory_recall "$payload"; then
  : > "$(sentinel_path "recall" "$session_id")"
fi

# Tools whose output is prose or logs, and therefore compressible without
# breaking the agent's reasoning. Override with a comma-separated list.
# `Read`/`Edit` must never be added: their value IS the exact bytes.
allowed="${VELESDB_HOOK_COMPRESS_TOOLS:-Bash,Grep,WebFetch}"
case ",${allowed}," in
  *",${tool_name},"*) ;;
  *) passthrough ;;
esac

# `tool_response` is a string for some tools and an object for others (Bash
# reports `{stdout, stderr, …}`). Either way the compiler must receive TEXT
# WITH ITS LINE BREAKS.
#
# This used to be `tojson` for the object case, and that silently destroyed
# the result it was meant to shrink. A JSON encoding is a SINGLE line with
# `\n` escaped inside a string, so the segmenter had nothing to split on: it
# could neither deduplicate nor rank, and fell back to truncating from the
# head — keeping repeated build noise and dropping the error underneath it.
# Measured on a 55 KB `cargo` log: the compiled view retained 2048 characters
# of identical "Compiling …" lines and lost the `error[E0463]`, the
# `file.rs:412` location, a `do NOT run cargo clean` warning and the failing
# test name. Same log passed as raw text compiles to 121 characters WITH the
# error kept. It hit `Bash` hardest — the highest-volume tool of the three.
#
# So: collect every string leaf and join them with real newlines. Recursive
# rather than top-level, since a tool may nest its output one level down.
text="$(printf '%s' "$payload" \
  | jq -r '
      if (.tool_response | type) == "string" then .tool_response
      else [.tool_response | .. | strings] | join("\n")
      end')"
[ -n "$text" ] || passthrough

# Below the threshold, compiling costs more (a process spawn on every tool
# call) than the tokens it would save.
min_bytes="${VELESDB_HOOK_MIN_BYTES:-12000}"
original_bytes="$(printf '%s' "$text" | wc -c | tr -d ' ')"
[ "$original_bytes" -ge "$min_bytes" ] || passthrough

bin="${VELESDB_MEMORY_BIN:-velesdb-memory}"
command -v "$bin" >/dev/null 2>&1 || passthrough

# Capability probe, cached once per session per binary: a velesdb-memory
# released before `compile-stdin` ignores the subcommand and starts the MCP
# server instead. Probing with a tiny corpus under the watchdog tells the two
# apart without any version guessing, and without risking a hang.
probe_key="$(printf '%s' "$bin" | tr -c 'a-zA-Z0-9' '_')"
probe_marker="$(sentinel_path "compile-stdin-${probe_key}" "$session_id")"
if [ ! -f "$probe_marker" ]; then
  probe_out="$(sentinel_path "compile-stdin-probe-out-${probe_key}" "$session_id")"
  if printf 'probe corpus for the capability check\n' \
    | run_with_watchdog "${VELESDB_HOOK_PROBE_TIMEOUT:-10}" "$probe_out" "$bin" compile-stdin --budget 4096 \
    && jq -e '
      has("content")
      and (.risk == "low" or .risk == "medium" or .risk == "high")
    ' "$probe_out" >/dev/null 2>&1; then
    echo "yes" > "$probe_marker"
  else
    echo "no" > "$probe_marker"
  fi
  rm -f "$probe_out"
fi
[ "$(cat "$probe_marker")" = "yes" ] || passthrough

# Rule 1: the original survives, at a path the agent can read back.
archive_dir="${TMPDIR:-/tmp}/velesdb-agent-hooks/tool-output"
mkdir -p "$archive_dir"
archive="${archive_dir}/${session_id}-${tool_use_id}.txt"
printf '%s' "$text" > "$archive"

budget="${VELESDB_HOOK_TOKEN_BUDGET:-2000}"
budget_max="${VELESDB_HOOK_TOKEN_BUDGET_MAX:-$((budget * 2))}"
final_budget="$budget"
compiled_file="$(sentinel_path "compile-stdin-result" "${session_id}-${tool_use_id}")"

# One compilation attempt at $1 tokens, into $compiled_file.
compile_at() {
  printf '%s' "$text" \
    | run_with_watchdog 20 "$compiled_file" "$bin" compile-stdin --budget "$1" \
      --query "$tool_name output"
}

field() {
  jq -r "$1 // empty" "$compiled_file" 2>/dev/null || true
}

if ! compile_at "$budget"; then
  rm -f "$compiled_file"
  passthrough
fi
risk="$(field '.risk')"

# Rule 5: FIDELITY. `risk: high` is not a hint that the summary reads badly —
# it is the compiler reporting that at least one fragment it classifies as
# CRITICAL (a code fence, a negative constraint, an exact value, a URL) did not
# survive into the output verbatim. Shipping that in place of the real result
# is how a hook that exists to save tokens ends up costing a diagnosis.
#
# One escalation first, because the budget is usually what is too tight rather
# than the content being incompressible: a 268 KB cargo log is `high` at 2 000
# tokens and `medium` at 4 000. When the ceiling does not rescue it — a 584 KB
# thread-stack sample stays `high` at 2 000, 4 000, 8 000 and 16 000 — the
# answer is to compress nothing.
if [ "$risk" = "high" ] && [ "$budget_max" -gt "$budget" ]; then
  if ! compile_at "$budget_max"; then
    rm -f "$compiled_file"
    passthrough
  fi
  final_budget="$budget_max"
  risk="$(field '.risk')"
fi

# Fail closed on the wire contract. Only the two explicitly shippable verdicts
# may replace a tool result. A missing field, a renamed enum value, or a value
# of the wrong JSON type is not evidence that fidelity is acceptable.
case "$risk" in
  low|medium) ;;
  high)
    rm -f "$compiled_file"
    printf 'velesdb: declined to compile a %s-byte %s result — risk=high even at budget %s; the original is untouched\n' \
      "$original_bytes" "$tool_name" "$final_budget" >&2
    passthrough
    ;;
  *)
    rm -f "$compiled_file"
    printf 'velesdb: declined to compile a %s-byte %s result — missing or unsupported fidelity risk; the original is untouched\n' \
      "$original_bytes" "$tool_name" >&2
    passthrough
    ;;
esac

content="$(field '.content')"
tokens_in="$(jq -r '.tokens_in // 0' "$compiled_file" 2>/dev/null || echo 0)"
tokens_out="$(jq -r '.tokens_out // 0' "$compiled_file" 2>/dev/null || echo 0)"
tokens_saved="$(jq -r '.tokens_saved // 0' "$compiled_file" 2>/dev/null || echo 0)"
rm -f "$compiled_file"

# An empty compilation would replace a real result with nothing. compile-stdin
# already refuses to emit one, but the hook does not take that on trust.
[ -n "$content" ] || passthrough

footer="$(printf '\n\n--- velesdb: compiled %s tokens down to %s (saved %s, fidelity risk %s). Nothing was deleted — the untouched %s-byte original is at %s; Read it if this view is not enough. ---' \
  "$tokens_in" "$tokens_out" "$tokens_saved" "$risk" "$original_bytes" "$archive")"

jq -n --arg out "${content}${footer}" \
  '{hookSpecificOutput: {hookEventName: "PostToolUse", updatedToolOutput: $out}}'
