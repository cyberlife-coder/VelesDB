#!/usr/bin/env bash
# Bring the running velesdb-memory daemon up to a source tree's version.
#
# Build, install, restart, and VERIFY over the wire — the last step is the
# point: `cargo install` replacing a file proves nothing about what is
# actually serving, because the service manager keeps the old process alive on
# its own inode until it is restarted.
#
# Safe to run mid-session since 0.11.4: the daemon drains in-flight MCP
# sessions on SIGTERM instead of dropping them. Before that it killed live
# connections, which is exactly how a "quick update" used to break the very
# session that ran it.
#
# The source tree is NEVER guessed from a directory layout — `VELESDB_REPO`,
# or the current directory when it is itself the tree. A default pointing at
# one contributor's folders is how a shipped script works on exactly one
# machine.
set -euo pipefail

REPO="${VELESDB_REPO:-$PWD}"
URL="${VELESDB_MCP_URL:-https://127.0.0.1:18090/mcp}"
# Same label `scripts/install-memory-daemon.sh` registers the service under.
LABEL="${VELESDB_SERVICE_LABEL:-com.velesdb.memory}"

running_version() {
  curl -sk --max-time 5 "$URL" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"update","version":"1"}}}' \
    2>/dev/null | grep -o '"version":"[0-9][0-9.]*"' | grep -o '[0-9][0-9.]*' | head -1
}

manifest="$REPO/crates/velesdb-memory/Cargo.toml"
if [ ! -f "$manifest" ]; then
  echo "not a velesdb source tree: $REPO" >&2
  echo "run this from the source tree, or set VELESDB_REPO to its path" >&2
  exit 1
fi

before=$(running_version || true)
target=$(grep -m1 '^version' "$manifest" | cut -d'"' -f2)
echo "daemon: ${before:-unreachable} → building ${target} from ${REPO}"

# The daemon's own features, not the crate defaults: http for the shared
# transport every client joins, ollama for the semantic embedder, extract for
# autograph. Installing without them yields a daemon that starts and quietly
# does less.
cargo install --path "$REPO/crates/velesdb-memory" \
  --bin velesdb-memory \
  --features http,ollama,extract \
  --force

if command -v launchctl >/dev/null 2>&1; then
  launchctl kickstart -k "gui/$(id -u)/${LABEL}"
elif command -v systemctl >/dev/null 2>&1; then
  systemctl --user restart "${LABEL}"
else
  echo "no launchctl or systemctl found — restart the daemon yourself, then re-run" >&2
  exit 1
fi

for _ in $(seq 1 20); do
  after=$(running_version || true)
  [ -n "${after:-}" ] && break
  sleep 1
done

if [ -z "${after:-}" ]; then
  echo "daemon did not answer after the restart — inspect the ${LABEL} service" >&2
  exit 1
fi
if [ "$after" != "$target" ]; then
  echo "daemon answers ${after}, expected ${target} — the restart did not pick up the new binary" >&2
  exit 1
fi

# The freshness cache is keyed on time, not on version: drop it so the next
# session re-reads the registry instead of repeating a now-stale verdict.
rm -f "$HOME/.velesdb-memory/.latest-version" 2>/dev/null || true
echo "daemon now serving ${after}, verified over the wire"
