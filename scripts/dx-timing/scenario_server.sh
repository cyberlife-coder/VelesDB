#!/usr/bin/env bash
# Scenario D — cargo install velesdb-server + REST hello-world.
# Replaces the previous Docker scenario (no public image is published yet — see
# timing-results.md "Honesty notes"). This path measures "developer has the Rust
# toolchain, cargo-installs the binary from crates.io, talks to it via HTTP".

set -euo pipefail

START=$(date +%s.%N)

cargo install --quiet --locked velesdb-server@3.8.0

DATA_DIR=$(mktemp -d -t velesdb_dx_server_XXXX)
trap 'rm -rf "$DATA_DIR"' EXIT

velesdb-server --data-dir "$DATA_DIR" --port 18080 >/tmp/velesdb-server.log 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true; rm -rf "$DATA_DIR"' EXIT

# Wait for /health, bounded. Track success explicitly so the loop's
# silent fall-through cannot hide a server that never came up: if the
# health probe never succeeds within the 60-second budget, abort with
# an explicit diagnostic instead of letting the next curl call fail
# with a cryptic 'connection refused'.
HEALTH_OK=0
for _ in $(seq 1 60); do
    if curl -fsS http://127.0.0.1:18080/health >/dev/null 2>&1; then
        HEALTH_OK=1
        break
    fi
    sleep 1
done
if [ "$HEALTH_OK" -ne 1 ]; then
    echo "velesdb-server did not become healthy within 60 s. Server log:" >&2
    sed 's/^/  | /' /tmp/velesdb-server.log >&2 || true
    exit 1
fi

# Hello-world via REST. Routes confirmed against
# crates/velesdb-server/src/routes.rs (POST upsert + POST search,
# /v1 prefix mounted in main.rs:155).
curl -fsS -X POST http://127.0.0.1:18080/v1/collections \
    -H 'content-type: application/json' \
    -d '{"name":"hello","dimension":4}' >/dev/null

curl -fsS -X POST http://127.0.0.1:18080/v1/collections/hello/points \
    -H 'content-type: application/json' \
    -d '{"points":[{"id":1,"vector":[0.1,0.2,0.3,0.4]},{"id":2,"vector":[0.5,0.6,0.7,0.8]}]}' >/dev/null

curl -fsS -X POST http://127.0.0.1:18080/v1/collections/hello/search \
    -H 'content-type: application/json' \
    -d '{"vector":[0.1,0.2,0.3,0.4],"top_k":2}' \
    | grep -q '"id":"1"' || { echo "search did not return id=\"1\""; exit 1; }

END=$(date +%s.%N)
ELAPSED=$(awk "BEGIN {printf \"%.2f\", $END - $START}")
echo "SERVER_REST $ELAPSED"
