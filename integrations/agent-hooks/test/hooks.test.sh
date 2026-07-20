#!/usr/bin/env bash
# Test harness for the Claude Code agent hooks (session-start.sh, stop.sh,
# pre-compact.sh). Simulates the stdin JSON payloads Claude Code sends for
# each event and asserts the exact JSON shape each script prints back.
#
# Run: bash test/hooks.test.sh   (exit 0 = all good, exit 1 = a check failed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOKS_DIR="$ROOT/claude-code/hooks"

FAILED=0

pass() { printf 'ok - %s\n' "$1"; }
fail() { printf 'not ok - %s\n' "$1"; FAILED=1; }

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
# No hardcoded absolute user paths in the scripts (everything must come from
# the stdin payload or the .velesdb-hooks.json config).
# ---------------------------------------------------------------------------
if grep -rEn '/Users/[A-Za-z0-9_.-]+|/home/[A-Za-z0-9_.-]+' "$HOOKS_DIR" >/dev/null 2>&1; then
  fail "no hardcoded user home paths in hook scripts"
  grep -rEn '/Users/[A-Za-z0-9_.-]+|/home/[A-Za-z0-9_.-]+' "$HOOKS_DIR" >&2 || true
else
  pass "no hardcoded user home paths in hook scripts"
fi

# ---------------------------------------------------------------------------
# Static analysis via shellcheck, if available (gate says: note if not
# installed, don't fail the suite over its absence)
# ---------------------------------------------------------------------------
if command -v shellcheck >/dev/null 2>&1; then
  if find "$HOOKS_DIR" -name '*.sh' -print0 | xargs -0 shellcheck; then
    pass "shellcheck: hook scripts are clean"
  else
    fail "shellcheck: hook scripts are clean"
  fi
else
  echo "note - shellcheck not installed, skipping static analysis check"
fi

if [ "$FAILED" -ne 0 ]; then
  echo "FAILURES DETECTED"
  exit 1
fi

echo "All hook tests passed."
exit 0
