#!/usr/bin/env bash
# Test harness for every agent hook shipped here: Claude Code
# (session-start.sh, stop.sh, pre-compact.sh, post-tool-use.sh), Windsurf
# (pre-user-prompt.sh) and Codex CLI (session-start.sh, stop.sh). Simulates
# the stdin JSON payload each harness documents for each event and asserts
# the exact JSON shape the script prints back.
#
# Run: bash test/hooks.test.sh   (exit 0 = all good, exit 1 = a check failed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOKS_DIR="$ROOT/claude-code/hooks"
WINDSURF_HOOKS_DIR="$ROOT/windsurf/hooks"

FAILED=0

pass() { printf 'ok - %s\n' "$1"; }
fail() { printf 'not ok - %s\n' "$1"; FAILED=1; }

# Assert that a block of text injected into a MODEL's context describes
# load_working_context's return value correctly.
#
# Asserting only that the text mentions the tool NAME — which is all this
# harness did while every one of these three scripts still carried the stale
# "if it returns null, nothing was saved yet" instruction — cannot catch the
# defect that matters here. The tool never returns null: its output schema is
# an object whose only required key is 'found'. A model told to look for null
# never detects a miss, never reads other_sessions, and starts fresh on top of
# work that a one-character typo in the session id hid from it.
#
# So the check has two halves, and the negative one is what survives a revert:
# the envelope's fields must be named, AND the null instruction must be gone.
assert_envelope_contract() {
  label="$1"
  text="$2"
  for field in found working other_sessions; do
    if printf '%s' "$text" | grep -q "$field"; then
      pass "$label: names '$field'"
    else
      fail "$label: names '$field'"
    fi
  done
  if printf '%s' "$text" | grep -qi "returns null"; then
    fail "$label: must not tell the model a null result means nothing was saved"
  else
    pass "$label: does not tell the model to expect a null result"
  fi
}

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required to run this test harness" >&2
  exit 1
fi

TMP_TEST_DIR="$(mktemp -d)"
# shellcheck disable=SC2329 # invoked indirectly via `trap ... EXIT` below
cleanup() { rm -rf "$TMP_TEST_DIR"; }
trap cleanup EXIT

# Isolate the sentinel-file mechanism from the real /tmp so repeated runs
# never see stale sentinels from a previous run or a real session.
export TMPDIR="$TMP_TEST_DIR/tmp"
mkdir -p "$TMPDIR"

PROJECT_DIR="$TMP_TEST_DIR/project"
mkdir -p "$PROJECT_DIR"
cat > "$PROJECT_DIR/.velesdb-hooks.json" <<'EOF'
{"project": "test-project", "session": "rolling"}
EOF

SESSION_ID="test-session-$$"

# ---------------------------------------------------------------------------
# SessionStart
# ---------------------------------------------------------------------------
session_start_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg sid "$SESSION_ID" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "SessionStart", source: "startup"}')"

session_start_out="$(printf '%s' "$session_start_payload" | bash "$HOOKS_DIR/session-start.sh")"

if printf '%s' "$session_start_out" | jq -e '.hookSpecificOutput.hookEventName == "SessionStart"' >/dev/null; then
  pass "SessionStart: hookSpecificOutput.hookEventName is SessionStart"
else
  fail "SessionStart: hookSpecificOutput.hookEventName is SessionStart"
fi

if printf '%s' "$session_start_out" | jq -e '.hookSpecificOutput.additionalContext | contains("load_working_context")' >/dev/null; then
  pass "SessionStart: additionalContext mentions load_working_context"
else
  fail "SessionStart: additionalContext mentions load_working_context"
fi

assert_envelope_contract "SessionStart: additionalContext" \
  "$(printf '%s' "$session_start_out" | jq -r '.hookSpecificOutput.additionalContext')"

if printf '%s' "$session_start_out" | jq -e '.hookSpecificOutput.additionalContext | contains("test-project")' >/dev/null; then
  pass "SessionStart: additionalContext uses project from .velesdb-hooks.json"
else
  fail "SessionStart: additionalContext uses project from .velesdb-hooks.json"
fi

# ---------------------------------------------------------------------------
# Stop — first call blocks, second call (same session_id) passes
# ---------------------------------------------------------------------------
stop_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg sid "$SESSION_ID" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "Stop", last_assistant_message: "done"}')"

stop_out_1="$(printf '%s' "$stop_payload" | bash "$HOOKS_DIR/stop.sh")"

if printf '%s' "$stop_out_1" | jq -e '.decision == "block"' >/dev/null; then
  pass "Stop: first call blocks (decision == block)"
else
  fail "Stop: first call blocks (decision == block)"
fi

if printf '%s' "$stop_out_1" | jq -e '.reason | contains("save_working_context")' >/dev/null; then
  pass "Stop: reason mentions save_working_context"
else
  fail "Stop: reason mentions save_working_context"
fi

stop_out_2="$(printf '%s' "$stop_payload" | bash "$HOOKS_DIR/stop.sh")"

if printf '%s' "$stop_out_2" | jq -e '.decision == null' >/dev/null; then
  pass "Stop: second call in same session does not block"
else
  fail "Stop: second call in same session does not block"
fi

# A different session_id must get its own reminder (sentinel is per-session).
other_stop_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg sid "${SESSION_ID}-other" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "Stop", last_assistant_message: "done"}')"
other_stop_out="$(printf '%s' "$other_stop_payload" | bash "$HOOKS_DIR/stop.sh")"

if printf '%s' "$other_stop_out" | jq -e '.decision == "block"' >/dev/null; then
  pass "Stop: a different session_id gets its own first-call block"
else
  fail "Stop: a different session_id gets its own first-call block"
fi

# ---------------------------------------------------------------------------
# PreCompact — first call blocks, second call (same session_id) passes
# ---------------------------------------------------------------------------
pre_compact_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg sid "$SESSION_ID" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "PreCompact", trigger: "auto"}')"

pre_compact_out_1="$(printf '%s' "$pre_compact_payload" | bash "$HOOKS_DIR/pre-compact.sh")"

if printf '%s' "$pre_compact_out_1" | jq -e '.decision == "block"' >/dev/null; then
  pass "PreCompact: first call blocks (decision == block)"
else
  fail "PreCompact: first call blocks (decision == block)"
fi

if printf '%s' "$pre_compact_out_1" | jq -e '.reason | contains("save_working_context")' >/dev/null; then
  pass "PreCompact: reason mentions save_working_context"
else
  fail "PreCompact: reason mentions save_working_context"
fi

if printf '%s' "$pre_compact_out_1" | jq -e '.reason | contains("compile_transcript")' >/dev/null; then
  pass "PreCompact: reason mentions compile_transcript (V2b roadmap item, now shipped)"
else
  fail "PreCompact: reason mentions compile_transcript (V2b roadmap item, now shipped)"
fi

if printf '%s' "$pre_compact_out_1" | jq -e 'has("hookSpecificOutput") | not' >/dev/null; then
  pass "PreCompact: no hookSpecificOutput wrapper (unsupported for this event)"
else
  fail "PreCompact: no hookSpecificOutput wrapper (unsupported for this event)"
fi

pre_compact_out_2="$(printf '%s' "$pre_compact_payload" | bash "$HOOKS_DIR/pre-compact.sh")"

if printf '%s' "$pre_compact_out_2" | jq -e '. == {}' >/dev/null; then
  pass "PreCompact: second call in same session passes through ({})"
else
  fail "PreCompact: second call in same session passes through ({})"
fi

# ---------------------------------------------------------------------------
# Defaults when no .velesdb-hooks.json is present
# ---------------------------------------------------------------------------
NO_CONFIG_DIR="$TMP_TEST_DIR/no-config-project"
mkdir -p "$NO_CONFIG_DIR"
no_config_sid="test-session-nocfg-$$"

no_config_payload="$(jq -n --arg cwd "$NO_CONFIG_DIR" --arg sid "$no_config_sid" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "SessionStart", source: "startup"}')"

no_config_out="$(printf '%s' "$no_config_payload" | bash "$HOOKS_DIR/session-start.sh")"

if printf '%s' "$no_config_out" | jq -e '.hookSpecificOutput.additionalContext | contains("no-config-project")' >/dev/null; then
  pass "SessionStart: defaults project to basename(cwd) with no config file"
else
  fail "SessionStart: defaults project to basename(cwd) with no config file"
fi

if printf '%s' "$no_config_out" | jq -e '.hookSpecificOutput.additionalContext | contains("rolling")' >/dev/null; then
  pass "SessionStart: defaults session to \"rolling\" with no config file"
else
  fail "SessionStart: defaults session to \"rolling\" with no config file"
fi

# ---------------------------------------------------------------------------
# Windsurf pre_user_prompt — first call with a trajectory_id reminds, second
# call with the SAME trajectory_id is silent (single-event fold of the
# Claude Code load+save reminder, since Windsurf has no Stop/PreCompact).
# ---------------------------------------------------------------------------
WINDSURF_TRAJECTORY_ID="test-trajectory-$$"
windsurf_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg tid "$WINDSURF_TRAJECTORY_ID" \
  '{trajectory_id: $tid, cwd: $cwd, execution_id: "exec-1", model_name: "test-model"}')"

windsurf_out_1="$(printf '%s' "$windsurf_payload" | bash "$WINDSURF_HOOKS_DIR/pre-user-prompt.sh")"

if printf '%s' "$windsurf_out_1" | grep -q "load_working_context"; then
  pass "Windsurf pre_user_prompt: first call mentions load_working_context"
else
  fail "Windsurf pre_user_prompt: first call mentions load_working_context"
fi

assert_envelope_contract "Windsurf pre_user_prompt: first call" "$windsurf_out_1"

if printf '%s' "$windsurf_out_1" | grep -q "save_working_context"; then
  pass "Windsurf pre_user_prompt: first call also mentions save_working_context (no separate Stop event)"
else
  fail "Windsurf pre_user_prompt: first call also mentions save_working_context (no separate Stop event)"
fi

if printf '%s' "$windsurf_out_1" | grep -q "test-project"; then
  pass "Windsurf pre_user_prompt: uses project from .velesdb-hooks.json"
else
  fail "Windsurf pre_user_prompt: uses project from .velesdb-hooks.json"
fi

windsurf_out_2="$(printf '%s' "$windsurf_payload" | bash "$WINDSURF_HOOKS_DIR/pre-user-prompt.sh")"

if [ -z "$windsurf_out_2" ]; then
  pass "Windsurf pre_user_prompt: second call in same trajectory is silent"
else
  fail "Windsurf pre_user_prompt: second call in same trajectory is silent"
fi

# ---------------------------------------------------------------------------
# Codex CLI SessionStart / Stop.
#
# The payloads below are built from the documented Codex stdin contract
# (learn.chatgpt.com/docs/hooks, checked 2026-07-25): the common fields
# session_id / transcript_path / cwd / hook_event_name / model /
# permission_mode, plus `source` on SessionStart and `stop_hook_active` +
# `last_assistant_message` on Stop. As with every other harness here, this
# asserts the scripts' decision logic against that contract — it does not and
# cannot prove that a real Codex build sends exactly these fields.
# ---------------------------------------------------------------------------
CODEX_HOOKS_DIR="$ROOT/codex/hooks"
CODEX_SESSION_ID="test-codex-session-$$"

codex_session_start_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg sid "$CODEX_SESSION_ID" \
  '{session_id: $sid, transcript_path: null, cwd: $cwd, hook_event_name: "SessionStart", model: "test-model", permission_mode: "default", source: "startup"}')"

codex_session_start_out="$(printf '%s' "$codex_session_start_payload" | bash "$CODEX_HOOKS_DIR/session-start.sh")"

if printf '%s' "$codex_session_start_out" | jq -e '.hookSpecificOutput.hookEventName == "SessionStart"' >/dev/null; then
  pass "Codex SessionStart: hookSpecificOutput.hookEventName is SessionStart"
else
  fail "Codex SessionStart: hookSpecificOutput.hookEventName is SessionStart"
fi

if printf '%s' "$codex_session_start_out" | jq -e '.hookSpecificOutput.additionalContext | contains("load_working_context") and contains("test-project")' >/dev/null; then
  pass "Codex SessionStart: additionalContext asks for load_working_context on the configured project"
else
  fail "Codex SessionStart: additionalContext asks for load_working_context on the configured project"
fi

assert_envelope_contract "Codex SessionStart: additionalContext" \
  "$(printf '%s' "$codex_session_start_out" | jq -r '.hookSpecificOutput.additionalContext')"

if printf '%s' "$codex_session_start_out" | jq -e '.hookSpecificOutput.additionalContext | contains("COMPACTION") | not' >/dev/null; then
  pass "Codex SessionStart: source=startup does not mention compaction"
else
  fail "Codex SessionStart: source=startup does not mention compaction"
fi

# source="compact" is the ONLY documented channel through which anything about
# a Codex compaction can reach the model — PreCompact/PostCompact support
# neither additionalContext nor a decision/reason. If this check ever goes
# missing, the compaction step silently drops off the Codex integration.
codex_compact_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg sid "$CODEX_SESSION_ID" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "SessionStart", source: "compact"}')"

codex_compact_out="$(printf '%s' "$codex_compact_payload" | bash "$CODEX_HOOKS_DIR/session-start.sh")"

if printf '%s' "$codex_compact_out" | jq -e '.hookSpecificOutput.additionalContext | contains("COMPACTION") and contains("save_working_context")' >/dev/null; then
  pass "Codex SessionStart: source=compact adds the post-compaction save reminder"
else
  fail "Codex SessionStart: source=compact adds the post-compaction save reminder"
fi

codex_stop_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg sid "$CODEX_SESSION_ID" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "Stop", stop_hook_active: false, last_assistant_message: "done"}')"

codex_stop_out_1="$(printf '%s' "$codex_stop_payload" | bash "$CODEX_HOOKS_DIR/stop.sh")"

if printf '%s' "$codex_stop_out_1" | jq -e '.decision == "block" and (.reason | contains("save_working_context"))' >/dev/null; then
  pass "Codex Stop: first call blocks and asks for save_working_context"
else
  fail "Codex Stop: first call blocks and asks for save_working_context"
fi

codex_stop_out_2="$(printf '%s' "$codex_stop_payload" | bash "$CODEX_HOOKS_DIR/stop.sh")"

if printf '%s' "$codex_stop_out_2" | jq -e '. == {}' >/dev/null; then
  pass "Codex Stop: second call in same session passes through ({})"
else
  fail "Codex Stop: second call in same session passes through ({})"
fi

# The Codex sentinel must not collide with the Claude Code one: both harnesses
# key on `session_id`, and a user running both would otherwise silence one.
codex_shared_id_payload="$(jq -n --arg cwd "$PROJECT_DIR" --arg sid "$SESSION_ID" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "Stop"}')"
codex_shared_id_out="$(printf '%s' "$codex_shared_id_payload" | bash "$CODEX_HOOKS_DIR/stop.sh")"

if printf '%s' "$codex_shared_id_out" | jq -e '.decision == "block"' >/dev/null; then
  pass "Codex Stop: sentinel is namespaced apart from the Claude Code Stop sentinel"
else
  fail "Codex Stop: sentinel is namespaced apart from the Claude Code Stop sentinel"
fi

codex_no_config_payload="$(jq -n --arg cwd "$NO_CONFIG_DIR" \
  '{session_id: "codex-nocfg", cwd: $cwd, hook_event_name: "SessionStart", source: "resume"}')"
codex_no_config_out="$(printf '%s' "$codex_no_config_payload" | bash "$CODEX_HOOKS_DIR/session-start.sh")"

if printf '%s' "$codex_no_config_out" | jq -e '.hookSpecificOutput.additionalContext | contains("no-config-project") and contains("rolling")' >/dev/null; then
  pass "Codex SessionStart: falls back to basename(cwd) + \"rolling\" with no config file"
else
  fail "Codex SessionStart: falls back to basename(cwd) + \"rolling\" with no config file"
fi

# ---------------------------------------------------------------------------
# PostToolUse — the only hook that REPLACES what the model sees, so every
# check below is really a data-loss check. Driven against fake
# `velesdb-memory` binaries rather than a built one: the harness must stay
# hermetic, and what is under test is the hook's decision logic, not the
# compiler (that has its own Rust tests).
# ---------------------------------------------------------------------------
FAKE_BIN_DIR="$TMP_TEST_DIR/bin"
mkdir -p "$FAKE_BIN_DIR"

# Behaves like a compile-stdin-capable binary: consumes stdin, prints the
# result JSON.
cat > "$FAKE_BIN_DIR/fake-ok" <<'FAKE'
#!/usr/bin/env bash
cat >/dev/null
printf '{"content":"COMPILED SUMMARY","tokens_in":4000,"tokens_out":300,"tokens_saved":3700,"risk":"medium"}\n'
FAKE

# Behaves like a binary whose compilation failed (budget too small, bad input…).
cat > "$FAKE_BIN_DIR/fake-fail" <<'FAKE'
#!/usr/bin/env bash
cat >/dev/null
echo "compile-stdin: boom" >&2
exit 1
FAKE

# Behaves like a velesdb-memory RELEASED BEFORE compile-stdin: it ignores the
# subcommand and starts the MCP server, which sits on stdin forever. Without
# the watchdog this is a hung agent, so this is the most important case here.
cat > "$FAKE_BIN_DIR/fake-old" <<'FAKE'
#!/usr/bin/env bash
sleep 30
FAKE

# Behaves like a corpus the compiler cannot fit without dropping something it
# classifies as CRITICAL — a code fence, an exact value, a URL. No budget
# rescues it; the real 584 KB thread-stack sample under
# .investigation/http-deadlock-2026-07-22/ measures `high` at 2 000, 4 000,
# 8 000 and 16 000.
cat > "$FAKE_BIN_DIR/fake-high" <<'FAKE'
#!/usr/bin/env bash
cat >/dev/null
printf '{"content":"LOSSY SUMMARY","tokens_in":4000,"tokens_out":300,"tokens_saved":3700,"risk":"high"}\n'
FAKE

# Behaves like a corpus that is merely CRAMPED rather than incompressible: the
# first budget loses something critical, twice the budget does not. That is the
# common case — the real 268 KB cargo log measures `high` at 2 000 and `medium`
# at 4 000 — and it is why the hook escalates before it refuses.
cat > "$FAKE_BIN_DIR/fake-escalate" <<'FAKE'
#!/usr/bin/env bash
cat >/dev/null
# argv: compile-stdin --budget N --query ...
if [ "${3:-0}" -ge 4000 ]; then
  printf '{"content":"ROOMIER SUMMARY","tokens_in":4000,"tokens_out":600,"tokens_saved":3400,"risk":"medium"}\n'
else
  printf '{"content":"CRAMPED SUMMARY","tokens_in":4000,"tokens_out":300,"tokens_saved":3700,"risk":"high"}\n'
fi
FAKE

chmod +x "$FAKE_BIN_DIR/fake-ok" "$FAKE_BIN_DIR/fake-fail" "$FAKE_BIN_DIR/fake-old" \
  "$FAKE_BIN_DIR/fake-high" "$FAKE_BIN_DIR/fake-escalate"

big_output="$(head -c 40000 < /dev/zero | tr '\0' 'x')"

post_tool_payload() {
  # $1 tool_name, $2 session suffix, $3 response text
  jq -n --arg cwd "$PROJECT_DIR" --arg sid "$SESSION_ID-$2" --arg tool "$1" --arg body "$3" \
    '{session_id: $sid, cwd: $cwd, hook_event_name: "PostToolUse", tool_name: $tool,
      tool_input: {command: "echo"}, tool_use_id: "toolu_test", tool_response: $body}'
}

# A tool NOT on the allowlist must be left strictly alone, however big it is.
# Read is the case that matters: the model needs file bytes verbatim.
read_out="$(post_tool_payload "Read" "read" "$big_output" \
  | VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-ok" bash "$HOOKS_DIR/post-tool-use.sh")"
if [ "$(printf '%s' "$read_out" | jq -c .)" = "{}" ]; then
  pass "PostToolUse: Read is never compressed, whatever its size"
else
  fail "PostToolUse: Read is never compressed, whatever its size"
fi

# Below the size threshold, compiling costs more than it saves.
small_out="$(post_tool_payload "Bash" "small" "tiny output" \
  | VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-ok" bash "$HOOKS_DIR/post-tool-use.sh")"
if [ "$(printf '%s' "$small_out" | jq -c .)" = "{}" ]; then
  pass "PostToolUse: output below the threshold is passed through untouched"
else
  fail "PostToolUse: output below the threshold is passed through untouched"
fi

# The nominal case: an allowlisted tool, over the threshold, capable binary.
big_out="$(post_tool_payload "Bash" "big" "$big_output" \
  | VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-ok" bash "$HOOKS_DIR/post-tool-use.sh")"

if printf '%s' "$big_out" | jq -e '.hookSpecificOutput.hookEventName == "PostToolUse"' >/dev/null; then
  pass "PostToolUse: replaces the result with hookEventName PostToolUse"
else
  fail "PostToolUse: replaces the result with hookEventName PostToolUse"
fi

if printf '%s' "$big_out" | jq -e '.hookSpecificOutput.updatedToolOutput | contains("COMPILED SUMMARY")' >/dev/null; then
  pass "PostToolUse: updatedToolOutput carries the compiled content"
else
  fail "PostToolUse: updatedToolOutput carries the compiled content"
fi

# --- Rule 5: fidelity ------------------------------------------------------
# `risk: high` is not "the summary reads badly". It is the compiler reporting
# that a fragment it classifies as critical did not survive verbatim — and on
# this path there is no store behind the `ctx://source/…` handles it mints, so
# the temp file is the ONLY way back. Shipping that in place of the real result
# is how a hook meant to save tokens costs a diagnosis.
high_out="$(post_tool_payload "Bash" "high" "$big_output" \
  | VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-high" bash "$HOOKS_DIR/post-tool-use.sh" 2>/dev/null)"
if [ "$(printf '%s' "$high_out" | jq -c .)" = "{}" ]; then
  pass "PostToolUse: a risk=high compilation is refused, leaving the result untouched"
else
  fail "PostToolUse: a risk=high compilation is refused, leaving the result untouched"
fi

# The positive control for the refusal above. Without it, a hook that refused
# EVERYTHING would satisfy it while never compressing anything — the same
# assertion passing for the opposite reason. `fake-ok` reports `medium`, and
# `$big_out` above shows it IS shipped.
if printf '%s' "$big_out" | jq -e '.hookSpecificOutput.updatedToolOutput | contains("COMPILED SUMMARY")' >/dev/null; then
  pass "PostToolUse: a risk=medium compilation IS shipped (control for the refusal)"
else
  fail "PostToolUse: a risk=medium compilation IS shipped (control for the refusal)"
fi

# Escalate before refusing: the budget is usually what is too tight, not the
# content that is incompressible.
esc_out="$(post_tool_payload "Bash" "escalate" "$big_output" \
  | VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-escalate" bash "$HOOKS_DIR/post-tool-use.sh" 2>/dev/null)"
if printf '%s' "$esc_out" | jq -e '.hookSpecificOutput.updatedToolOutput | contains("ROOMIER SUMMARY")' >/dev/null; then
  pass "PostToolUse: risk=high at the first budget retries at the ceiling and ships that"
else
  fail "PostToolUse: risk=high at the first budget retries at the ceiling and ships that"
fi

# ...and the cramped first attempt must NOT be what reaches the model, or the
# escalation would be decorative.
if printf '%s' "$esc_out" | jq -e '.hookSpecificOutput.updatedToolOutput | contains("CRAMPED SUMMARY") | not' >/dev/null; then
  pass "PostToolUse: the refused first attempt never reaches the model"
else
  fail "PostToolUse: the refused first attempt never reaches the model"
fi

# A ceiling equal to the starting budget means there is no second attempt to
# make, so the first `high` is final. This is what makes the ceiling a real
# bound rather than a suggestion.
noesc_out="$(post_tool_payload "Bash" "noesc" "$big_output" \
  | VELESDB_HOOK_TOKEN_BUDGET=2000 VELESDB_HOOK_TOKEN_BUDGET_MAX=2000 \
    VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-escalate" bash "$HOOKS_DIR/post-tool-use.sh" 2>/dev/null)"
if [ "$(printf '%s' "$noesc_out" | jq -c .)" = "{}" ]; then
  pass "PostToolUse: a ceiling equal to the budget forbids the retry and refuses"
else
  fail "PostToolUse: a ceiling equal to the budget forbids the retry and refuses"
fi

# The model must be told which fidelity it is reading. A compiled view that
# looks identical whether it lost something or not is one the model cannot
# reason about.
if printf '%s' "$big_out" | jq -e '.hookSpecificOutput.updatedToolOutput | contains("fidelity risk medium")' >/dev/null; then
  pass "PostToolUse: the footer names the fidelity risk of what it shipped"
else
  fail "PostToolUse: the footer names the fidelity risk of what it shipped"
fi

# --- Against the REAL binary -----------------------------------------------
# Everything above drives fake binaries, which is right for the decision logic
# but proves nothing about the contract the two sides actually share: that the
# compiler emits a `risk` field the hook can read, spelled the way the hook
# expects. A rename on either side would leave every test above green.
#
# Two corpora, both generated here so the harness stays hermetic, both measured
# against the installed binary:
#   * URLs and hex checksums — content the classifier calls CRITICAL. Dropping
#     any of it is `high`, and stays `high` well past the ceiling.
#   * identical heartbeat lines — the repetitive case abstraction handles
#     cleanly, and reports `medium`.
real_bin="${VELESDB_MEMORY_BIN_REAL:-velesdb-memory}"
if command -v "$real_bin" >/dev/null 2>&1 \
  && printf 'probe\n' | "$real_bin" compile-stdin --budget 4096 2>/dev/null | jq -e 'has("risk")' >/dev/null 2>&1; then

  critical_corpus="$(awk 'BEGIN { for (i = 0; i < 200; i++)
    printf "2026-08-04T00:%02d:00Z ERROR shard=%d url=https://example.invalid/api/v1/resource/%d checksum=0x%08x elapsed=%dms\n", i % 60, i, i, i, i * 7 }')"
  repetitive_corpus="$(awk 'BEGIN { for (i = 0; i < 400; i++)
    print "INFO  worker heartbeat ok, queue depth nominal, nothing to report" }')"

  real_high="$(post_tool_payload "Bash" "realhigh" "$critical_corpus" \
    | VELESDB_MEMORY_BIN="$real_bin" bash "$HOOKS_DIR/post-tool-use.sh" 2>/dev/null)"
  if [ "$(printf '%s' "$real_high" | jq -c .)" = "{}" ]; then
    pass "PostToolUse/real: the real compiler's risk=high verdict is honoured"
  else
    fail "PostToolUse/real: the real compiler's risk=high verdict is honoured"
  fi

  real_ok="$(post_tool_payload "Bash" "realok" "$repetitive_corpus" \
    | VELESDB_MEMORY_BIN="$real_bin" bash "$HOOKS_DIR/post-tool-use.sh" 2>/dev/null)"
  if printf '%s' "$real_ok" | jq -e '.hookSpecificOutput.updatedToolOutput | contains("fidelity risk")' >/dev/null; then
    pass "PostToolUse/real: a compressible corpus is still compressed by the real compiler"
  else
    fail "PostToolUse/real: a compressible corpus is still compressed by the real compiler"
  fi
else
  # Loud on purpose. A silent skip reads exactly like a pass, and the whole
  # point of this section is that the fakes cannot catch a contract change.
  printf '# SKIP - real-binary checks: no compile-stdin-capable velesdb-memory on PATH\n'
  printf '#        (set VELESDB_MEMORY_BIN_REAL to point at one)\n'
fi

# Rule 1 — nothing is deleted: the replacement must point at the untouched
# original, and that file must actually hold it.
archive_path="$(printf '%s' "$big_out" \
  | jq -r '.hookSpecificOutput.updatedToolOutput' \
  | sed -n 's/.*original is at \(.*\); Read it.*/\1/p')"
if [ -n "$archive_path" ] && [ -f "$archive_path" ]; then
  pass "PostToolUse: the replacement quotes a real path to the original"
else
  fail "PostToolUse: the replacement quotes a real path to the original"
fi

if [ -n "$archive_path" ] && [ -f "$archive_path" ] \
  && [ "$(wc -c < "$archive_path" | tr -d ' ')" = "$(printf '%s' "$big_output" | wc -c | tr -d ' ')" ]; then
  pass "PostToolUse: the archived original is byte-complete"
else
  fail "PostToolUse: the archived original is byte-complete"
fi

# A failing compilation must never cost the agent its tool result.
fail_out="$(post_tool_payload "Bash" "failbin" "$big_output" \
  | VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-fail" bash "$HOOKS_DIR/post-tool-use.sh")"
if [ "$(printf '%s' "$fail_out" | jq -c .)" = "{}" ]; then
  pass "PostToolUse: a failed compilation falls back to the untouched output"
else
  fail "PostToolUse: a failed compilation falls back to the untouched output"
fi

# No binary at all — the overwhelmingly common case before the release that
# ships compile-stdin.
missing_out="$(post_tool_payload "Bash" "nobin" "$big_output" \
  | VELESDB_MEMORY_BIN="$TMP_TEST_DIR/definitely-not-installed" bash "$HOOKS_DIR/post-tool-use.sh")"
if [ "$(printf '%s' "$missing_out" | jq -c .)" = "{}" ]; then
  pass "PostToolUse: a missing velesdb-memory binary falls back cleanly"
else
  fail "PostToolUse: a missing velesdb-memory binary falls back cleanly"
fi

# The hang case: an older binary treats our piped stdin as MCP traffic. The
# watchdog must bound it AND the hook must still answer.
old_started="$(date +%s)"
old_out="$(post_tool_payload "Bash" "oldbin" "$big_output" \
  | VELESDB_HOOK_PROBE_TIMEOUT=2 VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-old" bash "$HOOKS_DIR/post-tool-use.sh")"
old_elapsed=$(( $(date +%s) - old_started ))
if [ "$(printf '%s' "$old_out" | jq -c .)" = "{}" ]; then
  pass "PostToolUse: a pre-compile-stdin binary falls back instead of hanging"
else
  fail "PostToolUse: a pre-compile-stdin binary falls back instead of hanging"
fi
# Bound checked against the fake binary's own 30s sleep, NOT against the 2s
# probe timeout: what is under test is "the watchdog cut it short", and a
# tight wall-clock budget would just make this assertion fail on a loaded CI
# runner (observed once, with a full cargo test suite running alongside).
if [ "$old_elapsed" -lt 25 ]; then
  pass "PostToolUse: the watchdog bounds the probe (${old_elapsed}s < the fake binary's 30s)"
else
  fail "PostToolUse: the watchdog bounds the probe (${old_elapsed}s < the fake binary's 30s)"
fi

# ---------------------------------------------------------------------------
# No hardcoded absolute user paths in the scripts (everything must come from
# the stdin payload or the .velesdb-hooks.json config).
# ---------------------------------------------------------------------------
if grep -rEn '/Users/[A-Za-z0-9_.-]+|/home/[A-Za-z0-9_.-]+' "$HOOKS_DIR" "$WINDSURF_HOOKS_DIR" "$CODEX_HOOKS_DIR" >/dev/null 2>&1; then
  fail "no hardcoded user home paths in hook scripts"
  grep -rEn '/Users/[A-Za-z0-9_.-]+|/home/[A-Za-z0-9_.-]+' "$HOOKS_DIR" "$WINDSURF_HOOKS_DIR" "$CODEX_HOOKS_DIR" >&2 || true
else
  pass "no hardcoded user home paths in hook scripts"
fi

# ---------------------------------------------------------------------------
# --- The compiler must receive TEXT, with its line breaks -------------------
#
# `tool_response` is a STRING for some tools and an OBJECT for others — Bash,
# the highest-volume one, reports `{stdout, stderr, …}`. Every check above
# uses the string form, which is why this shipped broken: the object branch
# used to JSON-encode the response, handing the segmenter a SINGLE line with
# `\n` escaped inside it. With nothing to split on it could neither
# deduplicate nor rank, and truncated from the head — keeping repeated build
# noise and dropping the error underneath. On a real 55 KB cargo log that
# meant losing `error[E0463]`, the `file.rs:412` location, a `do NOT` warning
# and the failing test name, while emitting 2048 characters of identical
# "Compiling …" lines.
#
# So the guard asserts what the hook HANDS THE BINARY, not what comes back:
# multiple lines, and the unique line still present.
cat > "$FAKE_BIN_DIR/fake-record" <<FAKE
#!/usr/bin/env bash
cat > "$TMP_TEST_DIR/received.txt"
printf '{"content":"COMPILED","tokens_in":4000,"tokens_out":300,"tokens_saved":3700}\n'
FAKE
chmod +x "$FAKE_BIN_DIR/fake-record"

noisy_lines="$(for _ in $(seq 1 900); do echo "   Compiling velesdb-core v4.0.0"; done)"
buried="$noisy_lines
error[E0463]: can't find crate for \`core\`
$noisy_lines"

jq -n --arg cwd "$PROJECT_DIR" --arg sid "$SESSION_ID-obj" --arg body "$buried" \
  '{session_id: $sid, cwd: $cwd, hook_event_name: "PostToolUse", tool_name: "Bash",
    tool_input: {command: "cargo build"}, tool_use_id: "toolu_obj",
    tool_response: {stdout: $body, stderr: ""}}' \
  | VELESDB_MEMORY_BIN="$FAKE_BIN_DIR/fake-record" bash "$HOOKS_DIR/post-tool-use.sh" >/dev/null

received_lines="$(wc -l < "$TMP_TEST_DIR/received.txt" | tr -d ' ')"
if [ "$received_lines" -gt 1 ]; then
  pass "PostToolUse: an object tool_response reaches the compiler as real lines"
else
  fail "PostToolUse: an object tool_response reaches the compiler as real lines (got $received_lines line(s) — JSON-encoded?)"
fi

if grep -q "E0463" "$TMP_TEST_DIR/received.txt"; then
  pass "PostToolUse: the buried error line survives extraction"
else
  fail "PostToolUse: the buried error line survives extraction"
fi

# Static analysis via shellcheck, if available (gate says: note if not
# installed, don't fail the suite over its absence)
# ---------------------------------------------------------------------------
if command -v shellcheck >/dev/null 2>&1; then
  if find "$HOOKS_DIR" "$WINDSURF_HOOKS_DIR" "$CODEX_HOOKS_DIR" -name '*.sh' -print0 | xargs -0 shellcheck; then
    pass "shellcheck: hook scripts are clean"
  else
    fail "shellcheck: hook scripts are clean"
  fi
else
  echo "note - shellcheck not installed, skipping static analysis check"
fi

# No model-facing text anywhere in this directory may describe
# load_working_context as returning null.
# ---------------------------------------------------------------------------
# The three hook scripts are not the only text this directory injects into a
# model's context: codex/README.md ships a ready-made AGENTS.md block that
# users are told to paste verbatim, and it is the documented fallback for
# anyone who cannot install hooks. It carried the stale "a null result means
# nothing was saved yet" instruction for its whole life, because the checks
# above only ever read what the SCRIPTS printed. Scanning the tree — prose
# included — is what makes the next model-facing surface arrive covered
# instead of arriving stale.
# `--exclude-dir=test` keeps this harness from matching its own explanation
# above; nothing under test/ is ever injected into a model's context.
stale_null_claims="$(grep -rniE 'null result|returns null' "$ROOT" \
  --include='*.sh' --include='*.md' --exclude-dir=test || true)"
if [ -z "$stale_null_claims" ]; then
  pass "no agent-facing text tells a model load_working_context can return null"
else
  fail "agent-facing text still promises a null result: $stale_null_claims"
fi

if [ "$FAILED" -ne 0 ]; then
  echo "FAILURES DETECTED"
  exit 1
fi

echo "All hook tests passed."
exit 0
