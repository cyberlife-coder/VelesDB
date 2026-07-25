#!/usr/bin/env bash
# Shared helpers for the VelesDB Codex CLI hooks.
# Sourced by session-start.sh and stop.sh — not meant to be run directly.
#
# Kept byte-for-byte compatible with ../../claude-code/hooks/lib/common.sh and
# ../../windsurf/hooks/lib/common.sh on purpose: the Codex payload documents the
# same `session_id` and `cwd` field names as Claude Code, so the sentinel and
# config-resolution logic is genuinely identical rather than accidentally
# similar. Each harness keeps its own copy so a directory can be installed
# standalone (the install instructions copy one directory, not the tree).

# require_jq: fail loudly (not silently) if jq is missing, since every hook
# builds its JSON output through jq to get escaping right.
require_jq() {
  if ! command -v jq >/dev/null 2>&1; then
    echo "velesdb agent-hooks: 'jq' is required but was not found on PATH." >&2
    exit 1
  fi
}

# resolve_config CWD
# Walks up from CWD looking for a .velesdb-hooks.json file (project root
# convention). Sets PROJECT and SESSION globals. Falls back to
# project=basename(cwd) and session="rolling" when no config file is found
# or a field is missing — so the hooks work with zero setup, but a project
# can pin stable identifiers via the config file.
resolve_config() {
  local start_dir="$1"
  local dir="$start_dir"
  local config=""
  local depth=0

  while [ "$depth" -lt 20 ]; do
    if [ -f "$dir/.velesdb-hooks.json" ]; then
      config="$dir/.velesdb-hooks.json"
      break
    fi
    if [ "$dir" = "/" ] || [ -z "$dir" ]; then
      break
    fi
    dir="$(dirname "$dir")"
    depth=$((depth + 1))
  done

  PROJECT=""
  SESSION=""
  if [ -n "$config" ] && jq -e . "$config" >/dev/null 2>&1; then
    PROJECT="$(jq -r '.project // empty' "$config")"
    SESSION="$(jq -r '.session // empty' "$config")"
  fi

  if [ -z "$PROJECT" ]; then
    PROJECT="$(basename "$start_dir")"
  fi
  if [ -z "$SESSION" ]; then
    SESSION="rolling"
  fi
}

# read_stdin_payload: read the hook's JSON payload from stdin exactly once.
read_stdin_payload() {
  cat
}

# sentinel_path KIND SESSION_ID: path to the once-per-session marker file
# used by the Stop hook to fire its reminder exactly once.
# Uses $TMPDIR (falling back to /tmp) rather than a hardcoded path so it
# works unmodified on macOS and Linux, and namespaces under
# velesdb-agent-hooks/ to avoid colliding with unrelated temp files.
sentinel_path() {
  local kind="$1"
  local session_id="$2"
  local dir="${TMPDIR:-/tmp}/velesdb-agent-hooks"
  mkdir -p "$dir"
  printf '%s/%s-%s.marker' "$dir" "$kind" "$session_id"
}
