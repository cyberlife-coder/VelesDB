#!/usr/bin/env bash
# 01_quickstart.sh — the README's "first success in 60 seconds", start to finish.
#
# Creates a 4-dimensional cosine collection, upserts three points, and searches
# for the nearest two. Starts its own server on port 8081 in a temporary data
# directory and shuts it down again on exit.
#
# Usage:
#   ./01_quickstart.sh
#   VELESDB_SERVER_BIN=/path/to/velesdb-server ./01_quickstart.sh

set -euo pipefail

BIN="${VELESDB_SERVER_BIN:-velesdb-server}"
PORT="${PORT:-8081}"
BASE="http://127.0.0.1:${PORT}/v1"

command -v "$BIN" > /dev/null 2>&1 || {
  echo "velesdb-server not found. Install it with 'cargo install velesdb-server'," >&2
  echo "or point VELESDB_SERVER_BIN at a binary built with 'cargo build --release -p velesdb-server'." >&2
  exit 1
}

DATA_DIR="$(mktemp -d)"
SERVER_PID=""

cleanup() {
  # SIGTERM (the default signal) drains in-flight requests and flushes every
  # write-ahead log before the process exits.
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2> /dev/null || true
    wait "$SERVER_PID" 2> /dev/null || true
  fi
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

echo "==> Starting velesdb-server on port ${PORT} (data dir: ${DATA_DIR})"
"$BIN" --host 127.0.0.1 --port "$PORT" --data-dir "$DATA_DIR" > "${DATA_DIR}/server.log" 2>&1 &
SERVER_PID=$!

# The readiness probe answers 200 only once the engine has finished loading.
for _ in $(seq 1 60); do
  if curl -sf "${BASE}/ready" > /dev/null; then break; fi
  sleep 1
done
curl -sf "${BASE}/ready" > /dev/null || {
  echo "Server never became ready. Log:" >&2
  cat "${DATA_DIR}/server.log" >&2
  exit 1
}
echo "    ready"

echo
echo "==> 1. Create a 4-dimensional collection"
curl -sS -X POST "${BASE}/collections" \
  -H "Content-Type: application/json" \
  -d '{"name": "quickstart", "dimension": 4, "metric": "cosine"}'
echo

echo
echo "==> 2. Insert three points"
curl -sS -X POST "${BASE}/collections/quickstart/points" \
  -H "Content-Type: application/json" \
  -d '{"points": [
        {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "first"}},
        {"id": 2, "vector": [0.9, 0.4, 0.0, 0.0], "payload": {"title": "second"}},
        {"id": 3, "vector": [0.1, 0.9, 0.0, 0.0], "payload": {"title": "third"}}
      ]}'
echo

echo
echo "==> 3. Search for the nearest two vectors"
curl -sS -X POST "${BASE}/collections/quickstart/search" \
  -H "Content-Type: application/json" \
  -d '{"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 2}'
echo

echo
echo "==> 4. Read the collection back"
curl -sS "${BASE}/collections/quickstart/config"
echo

cat <<'EOF'

How to read the search response
-------------------------------
  {"results":[{"id":"1","score":1.0,...},{"id":"2","score":0.91...,...}]}

  - point 1 is the exact match, so its cosine score is 1.0;
  - point 2 is the nearest neighbour (the last decimals depend on the CPU);
  - point 3 is correctly excluded by top_k = 2;
  - ids come back as JSON strings because they are u64 server-side.

Anything else is a failure: a body carrying an "error" message plus a
"VELES-NNN" code means the request was rejected — see
docs/reference/ERROR_CODES.md.
EOF
