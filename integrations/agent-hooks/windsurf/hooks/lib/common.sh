#!/usr/bin/env bash
# Shared helpers for the VelesDB agent-hooks scripts.
# Sourced by session-start.sh, stop.sh, pre-compact.sh — not meant to be run directly.

umask 077

physical_policy_start() {
  local candidate="$1"
  local depth=0
  while [ ! -d "$candidate" ] && [ "$depth" -lt 40 ]; do
    [ "$candidate" = "/" ] && break
    candidate="$(dirname "$candidate")"
    depth=$((depth + 1))
  done
  [ -d "$candidate" ] || return 1
  (cd "$candidate" 2>/dev/null && pwd -P)
}

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
  local physical_start
  physical_start="$(physical_policy_start "$start_dir")" || return 1
  start_dir="$physical_start"
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

safe_marker_key() {
  local value="$1"
  local checksum
  checksum="$(printf '%s' "$value" | cksum)"
  printf '%s' "${checksum// /-}"
}

marker_base_dir() {
  local parent="${TMPDIR:-/tmp}"
  local dir="${parent}/velesdb-agent-hooks-${UID}"
  if [ -L "$dir" ]; then
    return 1
  fi
  if [ ! -e "$dir" ]; then
    mkdir -m 700 "$dir" 2>/dev/null || [ -d "$dir" ] || return 1
  fi
  [ -d "$dir" ] && [ ! -L "$dir" ] && [ -O "$dir" ] || return 1
  chmod 700 "$dir" || return 1
  printf '%s' "$dir"
}

# sentinel_path KIND SESSION_ID: path to the once-per-session marker file
# used by the prompt hook to fire its reminder exactly once.
sentinel_path() {
  local kind="$1"
  local session_id="$2"
  local dir
  local key
  dir="$(marker_base_dir)" || return 1
  key="$(safe_marker_key "$session_id")"
  printf '%s/%s-%s.marker' "$dir" "$kind" "$key"
}

touch_private_marker() {
  local path="$1"
  local tmp
  if [ -L "$path" ] || { [ -e "$path" ] && [ ! -f "$path" ]; }; then
    return 1
  fi
  tmp="$(mktemp "${path}.tmp.XXXXXX")" || return 1
  if ! : > "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  mv -f "$tmp" "$path"
}

valid_private_marker() {
  local path="$1"
  [ -f "$path" ] && [ ! -L "$path" ] && [ -O "$path" ]
}
