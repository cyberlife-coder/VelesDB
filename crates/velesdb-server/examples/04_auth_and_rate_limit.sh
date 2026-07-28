#!/usr/bin/env bash
# 04_auth_and_rate_limit.sh — Bearer API keys and the per-IP rate limiter.
#
# Starts a server with two API keys and a deliberately tiny rate limit, then
# shows what each guard accepts and rejects:
#
#   - /v1/health and /v1/ready stay public (orchestrator probes cannot carry
#     an Authorization header);
#   - every other route, /metrics included, requires `Authorization: Bearer <key>`;
#   - any of the configured keys works, which is what makes rotation possible:
#     add the new key, deploy, drop the old one;
#   - the limiter answers 429 with a `retry-after` header once the per-IP
#     budget is exhausted.
#
# Keys are passed through VELESDB_API_KEYS (comma-separated). They can also
# live in the [auth] section of velesdb.toml — see ./velesdb.toml.
#
# Usage:
#   ./04_auth_and_rate_limit.sh
#   VELESDB_SERVER_BIN=/path/to/velesdb-server ./04_auth_and_rate_limit.sh

set -euo pipefail

BIN="${VELESDB_SERVER_BIN:-velesdb-server}"
PORT="${PORT:-8084}"
BASE="http://127.0.0.1:${PORT}/v1"
ROOT="http://127.0.0.1:${PORT}"

OLD_KEY="sk-example-old"
NEW_KEY="sk-example-new"

command -v "$BIN" > /dev/null 2>&1 || {
  echo "velesdb-server not found; see examples/README.md." >&2
  exit 1
}

DATA_DIR="$(mktemp -d)"
SERVER_PID=""
cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2> /dev/null || true
    wait "$SERVER_PID" 2> /dev/null || true
  fi
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

echo "==> Starting the server with two API keys and --rate-limit 5"
VELESDB_API_KEYS="${OLD_KEY},${NEW_KEY}" \
  "$BIN" --host 127.0.0.1 --port "$PORT" --data-dir "$DATA_DIR" --rate-limit 5 \
  > "${DATA_DIR}/server.log" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 60); do
  if curl -sf "${BASE}/ready" > /dev/null; then break; fi
  sleep 1
done
curl -sf "${BASE}/ready" > /dev/null || {
  echo "Server never became ready. Log:" >&2
  cat "${DATA_DIR}/server.log" >&2
  exit 1
}

echo
echo "==> The probes are public — no header, still 200"
curl -sS -o /dev/stdout -w '  <- HTTP %{http_code} on /v1/health\n' "${BASE}/health"
curl -sS -o /dev/stdout -w '  <- HTTP %{http_code} on /v1/ready\n' "${BASE}/ready"

echo
echo "==> A data route without a key: 401"
curl -sS -o /dev/stdout -w '  <- HTTP %{http_code}\n' "${BASE}/collections" || true

echo
echo "==> The same route with a wrong key: still 401"
curl -sS -o /dev/stdout -w '  <- HTTP %{http_code}\n' \
  -H "Authorization: Bearer sk-not-a-real-key" "${BASE}/collections" || true

echo
echo "==> With the first configured key: 200"
curl -sS -o /dev/stdout -w '  <- HTTP %{http_code}\n' \
  -H "Authorization: Bearer ${OLD_KEY}" "${BASE}/collections"

echo
echo "==> With the second configured key: also 200 (this is how rotation works)"
curl -sS -o /dev/stdout -w '  <- HTTP %{http_code}\n' \
  -H "Authorization: Bearer ${NEW_KEY}" "${BASE}/collections"

echo
echo "==> Create a collection with the key, to prove writes go through the same gate"
curl -sS -X POST "${BASE}/collections" \
  -H "Authorization: Bearer ${NEW_KEY}" \
  -H "Content-Type: application/json" \
  -d '{"name": "secured", "dimension": 4, "metric": "cosine"}'
echo

echo
echo "==> /metrics is NOT public: it leaks operational detail, so it needs a key too"
curl -sS -o /dev/null -w '  <- HTTP %{http_code} without a key\n' "${ROOT}/metrics" || true
curl -sS -o /dev/null -w '  <- HTTP %{http_code} with a key\n' \
  -H "Authorization: Bearer ${NEW_KEY}" "${ROOT}/metrics"

echo
echo "==> Rate limiter: 30 authenticated requests in a row against a 5 req/s budget"
codes=""
for _ in $(seq 1 30); do
  code="$(curl -sS -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer ${NEW_KEY}" "${BASE}/collections" || true)"
  codes="${codes} ${code}"
done
echo "   status codes:${codes}"
echo "   (expect a run of 200 followed by 429 once the per-IP budget is spent;"
echo "    the 429 responses carry a retry-after header)"

cat <<'EOF'

What this proves, and what it does not
--------------------------------------
  - Authentication is a flat list of keys: no users, no roles, no per-collection
    scoping. Keys are read once at startup, so removing a key needs a restart.
    Rotation is therefore: add the new key -> restart -> switch clients -> drop
    the old key -> restart.
  - The rate limiter is per process and held in memory. Two replicas do not
    share a budget; put a shared limiter in front if you need a global one.
  - Nothing here is encrypted. Over anything but loopback, add TLS with
    --tls-cert and --tls-key (both are required together; a half-configured
    pair makes the process exit at startup instead of silently serving HTTP).

See docs/guides/SERVER_SECURITY.md.
EOF
