#!/usr/bin/env bash
# Is the running velesdb-memory daemon the latest published release?
#
# Sourced by session-start.sh. Prints ONE line of guidance when the daemon is
# behind, and nothing at all otherwise — silence is the normal case, so a
# session that is already current pays no attention cost.
#
# Three rules this file exists to respect:
#
#  1. **It must never fail the session.** Every step is best-effort: an
#     unreachable daemon, an offline registry, a missing tool — each returns
#     empty and the hook stays silent. `set -e` is deliberately NOT relied on
#     here; the caller's `set -euo pipefail` is neutralised per-command.
#  2. **It must not add a network round-trip to every session start.** The
#     crates.io answer is cached for a day; only the local daemon is queried
#     each time, over loopback, in well under a second.
#  3. **It must not update anything by itself.** A `cargo install` is ~90
#     seconds and needs the source tree; doing that while a session boots
#     would block the user for a minute with no warning. The hook reports;
#     the agent decides and acts.

# Where this file sits, so the notice below can name the updater beside it
# instead of a path guessed at authoring time. `BASH_SOURCE[0]` is this file
# even though it is *sourced*, which is exactly what makes it usable here.
VELESDB_FRESHNESS_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

VELESDB_FRESHNESS_CACHE="${HOME}/.velesdb-memory/.latest-version"
VELESDB_FRESHNESS_TTL_SECONDS=86400
# The daemon's shared HTTP transport, same default port as
# `scripts/install-memory-daemon.sh` writes into the service definition.
VELESDB_MCP_URL="${VELESDB_MCP_URL:-https://127.0.0.1:18090/mcp}"

# Version string the daemon answers with, or empty if it cannot be reached.
veles_running_version() {
  local body
  body=$(curl -sk --max-time 3 "$VELESDB_MCP_URL" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"freshness","version":"1"}}}' \
    2>/dev/null) || return 0
  # Anchored on the server NAME so a version string belonging to anything
  # else in the envelope (protocol, client echo) can never be mistaken for
  # the daemon's. The trailing quote is stripped with a second `grep -o`
  # over digits-and-dots, NOT with a `$` anchor: the match ends in `"`, so
  # anchoring at end-of-line silently yields nothing — which is how a
  # freshness check turns into a check that never fires.
  printf '%s' "$body" \
    | grep -o '"name":"velesdb-memory","version":"[0-9][0-9.]*"' \
    | grep -o '"[0-9][0-9.]*"$' \
    | tr -d '"' \
    | head -1
}

# Latest version on crates.io, cached for a day. Empty when offline — an
# offline session must behave exactly like an up-to-date one.
veles_latest_version() {
  local now cached_at
  now=$(date +%s)
  if [ -f "$VELESDB_FRESHNESS_CACHE" ]; then
    cached_at=$(sed -n '1p' "$VELESDB_FRESHNESS_CACHE" 2>/dev/null)
    if [ -n "${cached_at:-}" ] && [ $((now - cached_at)) -lt "$VELESDB_FRESHNESS_TTL_SECONDS" ]; then
      sed -n '2p' "$VELESDB_FRESHNESS_CACHE" 2>/dev/null
      return 0
    fi
  fi
  local latest
  latest=$(curl -s --max-time 4 -H 'User-Agent: velesdb-memory-freshness' \
    https://crates.io/api/v1/crates/velesdb-memory 2>/dev/null \
    | grep -o '"max_version":"[0-9][0-9.]*"' \
    | grep -o '[0-9][0-9.]*' \
    | head -1) || return 0
  [ -z "${latest:-}" ] && return 0
  mkdir -p "$(dirname "$VELESDB_FRESHNESS_CACHE")" 2>/dev/null || true
  printf '%s\n%s\n' "$now" "$latest" > "$VELESDB_FRESHNESS_CACHE" 2>/dev/null || true
  printf '%s' "$latest"
}

# `0` when $1 is strictly older than $2, comparing numerically field by field.
# A lexical compare would call 0.9.2 newer than 0.11.3 — a mistake already made
# once by hand on this very crate.
veles_version_lt() {
  [ "$1" = "$2" ] && return 1
  local smallest
  smallest=$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | head -1)
  [ "$smallest" = "$1" ]
}

# The guidance line, or nothing.
veles_freshness_notice() {
  local running latest
  running=$(veles_running_version) || return 0
  [ -z "${running:-}" ] && return 0
  latest=$(veles_latest_version) || return 0
  [ -z "${latest:-}" ] && return 0
  veles_version_lt "$running" "$latest" || return 0
  # SC2016: the backticks below are markdown for the model reading this line,
  # and the string is a printf FORMAT — it must stay single-quoted so `%s` is
  # substituted by printf rather than by the shell.
  # shellcheck disable=SC2016
  printf 'velesdb-memory freshness: the running daemon is %s, but %s is published. Bring it up to date EARLY in this session (before writing memories, so the session benefits from the fixes) by running `%s/update-daemon.sh` with VELESDB_REPO pointing at the source tree, then confirm the version it reports. The script installs from that tree, restarts the daemon, and verifies over the wire.' \
    "$running" "$latest" "$VELESDB_FRESHNESS_SCRIPT_DIR"
}
