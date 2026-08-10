#!/usr/bin/env bash
# PostToolUse hook: shrink an oversized tool result BEFORE it enters the
# agent's context, using the deterministic VelesDB context compiler.
#
# Why this hook is different from the three advisory hooks. SessionStart, Stop and
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
#   1. Nothing is deleted. The complete original Bash output object is
#      serialized to a temp JSON file and its path is quoted in the
#      replacement, so the agent can `Read` it whenever the compiled view is
#      not enough. This is the out-of-store equivalent of a retrieval handle.
#   2. Identity fallback everywhere. Missing `jq`, missing or too-old binary,
#      compilation error, empty compiled output, unreadable JSON — every one
#      of them emits `{}` and leaves the tool result exactly as it was.
#   3. Bounded. The compile call runs under a watchdog (see
#      lib/common.sh run_with_watchdog); a binary predating `compile-stdin`
#      would otherwise treat our piped stdin as MCP traffic and hang.
#   4. Tool schema allowlist. Only `Bash` is currently compressed because its
#      structured output shape is documented and contract-tested. `Read` and
#      `Edit` are deliberately NEVER in it: the model needs file contents
#      verbatim, byte for byte.
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

# Numeric tuning comes from the environment and reaches arithmetic expansion,
# loop bounds, or CLI arguments. Accept only small decimal integers so a typo
# cannot hang every PostToolUse (and shell arithmetic never reparses attacker-
# controlled expressions).
positive_decimal_at_most() {
  local value="$1"
  local maximum="$2"
  case "$value" in
    ''|*[!0-9]*) return 1 ;;
  esac
  [ "${#value}" -le 10 ] || return 1
  [ "$value" -gt 0 ] 2>/dev/null && [ "$value" -le "$maximum" ] 2>/dev/null
}

command -v jq >/dev/null 2>&1 || passthrough

payload="$(read_stdin_payload)"

# A malformed payload is not something to fail loudly on: it would mean
# every tool call in the session errors.
printf '%s' "$payload" | jq -e . >/dev/null 2>&1 || passthrough

tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty')"
session_id="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
[ -n "$cwd" ] || cwd="$PWD"

# This happens before the compiler allowlist and size checks: MCP recall
# results are normally small and are evidence for the learning-loop guard,
# not candidates for context compression.
resolve_config "$cwd"
if [ -n "$session_id" ] && successful_memory_recall "$payload"; then
  pending_status=2
  if pending_dir="$(record_dir_path "pending-recall" "$session_id")"; then
    if promote_pending_recall \
      "$pending_dir" "recall" "$session_id" "$payload"; then
      pending_status=0
    else
      pending_status=$?
    fi
  fi
  if [ "$pending_status" -ne 0 ]; then
    if { [ "$pending_status" -eq 1 ] || [ "$pending_status" -eq 3 ]; } \
      && learning_loop_enabled \
      && recall_targets_current_project "$payload"; then
      marker_id="$(learning_marker_identity "$session_id")"
      if marker_path="$(sentinel_path "recall" "$marker_id")"; then
        touch_private_marker "$marker_path" || true
      fi
    fi
  fi
fi
[ -n "$session_id" ] || session_id="unknown-session"

# `updatedToolOutput` is ignored by Claude when it does not match the built-in
# tool's output schema. Bash is the only enabled tool because its object shape
# is both officially documented and pinned below. The environment variable can
# disable it, but cannot opt an unverified tool schema into replacement.
allowed="${VELESDB_HOOK_COMPRESS_TOOLS:-Bash}"
case ",${allowed}," in
  *",${tool_name},"*) ;;
  *) passthrough ;;
esac
[ "$tool_name" = "Bash" ] || passthrough

printf '%s' "$payload" | jq -e '
  (.tool_response | type) == "object"
  and ((.tool_response.stdout? | type) == "string")
  and ((.tool_response.stderr? | type) == "string")
  and ((.tool_response.interrupted? | type) == "boolean")
  and ((.tool_response.isImage? | type) == "boolean")
  and (.tool_response.isImage == false)
' >/dev/null 2>&1 || passthrough

# Bash reports `{stdout, stderr, …}`. The compiler must receive those two text
# fields with real line breaks, not a JSON encoding of the whole object.
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
# error kept. It hit `Bash`, the highest-volume supported tool.
#
# Join stdout and stderr with a real newline while the complete structured
# object is preserved separately for recovery.
text="$(printf '%s' "$payload" \
  | jq -r '[.tool_response.stdout, .tool_response.stderr]
      | map(select(length > 0))
      | join("\n")')"
[ -n "$text" ] || passthrough

# Below the threshold, compiling costs more (a process spawn on every tool
# call) than the tokens it would save.
min_bytes="${VELESDB_HOOK_MIN_BYTES:-12000}"
positive_decimal_at_most "$min_bytes" 1000000000 || passthrough
original_bytes="$(printf '%s' "$text" | wc -c | tr -d ' ')"
[ "$original_bytes" -ge "$min_bytes" ] || passthrough

bin="${VELESDB_MEMORY_BIN:-velesdb-memory}"
command -v "$bin" >/dev/null 2>&1 || passthrough

# Capability probe, cached once per session per binary: a velesdb-memory
# released before `compile-stdin` ignores the subcommand and starts the MCP
# server instead. Probing with a tiny corpus under the watchdog tells the two
# apart without any version guessing, and without risking a hang.
probe_timeout="${VELESDB_HOOK_PROBE_TIMEOUT:-10}"
positive_decimal_at_most "$probe_timeout" 60 || passthrough
probe_key="$(safe_marker_key "$bin")"
if ! probe_marker="$(sentinel_path "compile-stdin-${probe_key}" "$session_id")"; then
  passthrough
fi
if valid_private_marker "$probe_marker"; then
  :
elif [ -e "$probe_marker" ] || [ -L "$probe_marker" ]; then
  passthrough
else
  if ! probe_out="$(private_temp_file "compile-stdin-probe-out-${probe_key}")"; then
    passthrough
  fi
  if printf 'probe corpus for the capability check\n' \
    | run_with_watchdog "$probe_timeout" "$probe_out" "$bin" compile-stdin --budget 4096 \
    && jq -e '
      has("content")
      and (.risk == "low" or .risk == "medium" or .risk == "high")
    ' "$probe_out" >/dev/null 2>&1; then
    write_private_marker "$probe_marker" "yes" || passthrough
  else
    write_private_marker "$probe_marker" "no" || passthrough
  fi
  rm -f "$probe_out"
fi
valid_private_marker "$probe_marker" || passthrough
[ "$(cat "$probe_marker")" = "yes" ] || passthrough

budget="${VELESDB_HOOK_TOKEN_BUDGET:-2000}"
positive_decimal_at_most "$budget" 1000000 || passthrough
budget_max="${VELESDB_HOOK_TOKEN_BUDGET_MAX:-$((budget * 2))}"
positive_decimal_at_most "$budget_max" 1000000 || passthrough
[ "$budget_max" -ge "$budget" ] || passthrough
final_budget="$budget"
if ! compiled_file="$(private_temp_file "compile-stdin-result")"; then
  passthrough
fi

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

# An empty compilation would replace a real result with nothing. compile-stdin
# already refuses to emit one, but the hook does not take that on trust.
if ! jq -e '(.content | type) == "string" and (.content | length) > 0' \
  "$compiled_file" >/dev/null 2>&1; then
  rm -f "$compiled_file"
  passthrough
fi

# Rule 1: archive only when replacement is still possible. Passthrough paths
# retain the host's original directly and need no duplicate temp copy.
archive_dir="$(marker_base_dir)/tool-output" || passthrough
[ -L "$archive_dir" ] && passthrough
mkdir -p "$archive_dir" || passthrough
if [ ! -d "$archive_dir" ] || [ -L "$archive_dir" ]; then
  passthrough
fi
chmod 700 "$archive_dir" || passthrough
archive="$(mktemp "${archive_dir}/velesdb-output.XXXXXX")" || passthrough
if ! printf '%s' "$payload" | jq '.tool_response' > "$archive"; then
  rm -f "$archive" "$compiled_file"
  passthrough
fi

footer="$(printf '\n\n--- velesdb: compiled %s tokens down to %s (saved %s before this footer, fidelity risk %s). Nothing was deleted — the complete original Bash output object is serialized as JSON at %s; Read it if this view is not enough. The compiler received %s bytes of combined stdout/stderr text. ---' \
  "$tokens_in" "$tokens_out" "$tokens_saved" "$risk" "$archive" "$original_bytes")"

# Never replace a result merely because compression was faithful. The footer
# also costs context, so require a conservative net margin: each footer byte is
# counted as if it were a whole token (an upper bound), then add the configured
# minimum. This prevents a roomy retry from increasing paid input tokens.
min_saved="${VELESDB_HOOK_MIN_SAVED_TOKENS:-128}"
case "$min_saved" in
  ''|*[!0-9]*) rm -f "$archive" "$compiled_file"; passthrough ;;
esac
if [ "${#min_saved}" -gt 7 ] || [ "$min_saved" -gt 1000000 ]; then
  rm -f "$archive" "$compiled_file"
  passthrough
fi
footer_bytes="$(printf '%s' "$footer" | wc -c | tr -d ' ')"
if ! jq -e \
  --argjson minimum "$((min_saved + footer_bytes))" '
    (.tokens_in | type) == "number"
    and (.tokens_out | type) == "number"
    and (.tokens_saved | type) == "number"
    and (.tokens_in >= 0 and .tokens_in == (.tokens_in | floor))
    and (.tokens_out >= 0 and .tokens_out == (.tokens_out | floor))
    and (.tokens_saved >= 0 and .tokens_saved == (.tokens_saved | floor))
    and (.tokens_out < .tokens_in)
    and (.tokens_saved == (.tokens_in - .tokens_out))
    and (.tokens_saved >= $minimum)
  ' "$compiled_file" >/dev/null 2>&1
then
  rm -f "$archive" "$compiled_file"
  passthrough
fi
rm -f "$compiled_file"

# Preserve the complete host-provided Bash object and modify only its two text
# fields. This keeps optional version-specific fields while satisfying Claude's
# documented `{stdout, stderr, interrupted, isImage}` schema.
printf '%s' "$payload" | jq --arg out "${content}${footer}" '
  {
    hookSpecificOutput: {
      hookEventName: "PostToolUse",
      updatedToolOutput: (.tool_response | .stdout = $out | .stderr = "")
    }
  }
'
