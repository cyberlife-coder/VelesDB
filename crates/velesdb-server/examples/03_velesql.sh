#!/usr/bin/env bash
# 03_velesql.sh — VelesQL over HTTP.
#
#   POST /v1/query          {"query": "...", "params": {...}}
#   POST /v1/query/explain  {"query": "...", "analyze": false}
#
# The target collection comes from the FROM clause inside the query string, not
# from the URL: /v1/query is a database-level endpoint. Bind parameters are
# referenced as $name in the statement and supplied as bare keys in "params".
#
# Usage:
#   ./03_velesql.sh
#   VELESDB_SERVER_BIN=/path/to/velesdb-server ./03_velesql.sh

set -euo pipefail

BIN="${VELESDB_SERVER_BIN:-velesdb-server}"
PORT="${PORT:-8083}"
BASE="http://127.0.0.1:${PORT}/v1"

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

"$BIN" --host 127.0.0.1 --port "$PORT" --data-dir "$DATA_DIR" > "${DATA_DIR}/server.log" 2>&1 &
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

echo "==> Create the collection and load four documents"
curl -sS -X POST "${BASE}/collections" \
  -H "Content-Type: application/json" \
  -d '{"name": "docs", "dimension": 4, "metric": "cosine"}'
echo

curl -sS -X POST "${BASE}/collections/docs/points" \
  -H "Content-Type: application/json" \
  -d '{"points": [
        {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Ownership", "category": "rust",     "year": 2021}},
        {"id": 2, "vector": [0.9, 0.3, 0.0, 0.0], "payload": {"title": "Lifetimes", "category": "rust",     "year": 2023}},
        {"id": 3, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "Vacuum",    "category": "postgres", "year": 2022}},
        {"id": 4, "vector": [0.0, 0.9, 0.3, 0.0], "payload": {"title": "Sharding",  "category": "postgres", "year": 2024}}
      ]}'
echo

echo
echo "==> Vector search expressed in VelesQL (NEAR + bind parameter)"
# shellcheck disable=SC2016  # $v is a VelesQL bind parameter, not a shell variable
curl -sS -X POST "${BASE}/query" \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM docs WHERE vector NEAR $v LIMIT 3",
       "params": {"v": [1.0, 0.0, 0.0, 0.0]}}'
echo

echo
echo "==> Same search, projected to three columns and filtered on a payload field"
# shellcheck disable=SC2016  # $v is a VelesQL bind parameter, not a shell variable
curl -sS -X POST "${BASE}/query" \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT id, title, year FROM docs WHERE vector NEAR $v AND category = '"'"'rust'"'"' LIMIT 2",
       "params": {"v": [1.0, 0.0, 0.0, 0.0]}}'
echo

echo
echo "==> Aggregation: how many documents per category"
curl -sS -X POST "${BASE}/query" \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT category, COUNT(*) FROM docs GROUP BY category"}'
echo

echo
echo "==> EXPLAIN the plan the engine would run (no execution)"
curl -sS -X POST "${BASE}/query/explain" \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM docs LIMIT 10"}'
echo

echo
echo "==> A syntax error, so you can recognise the rejection shape"
curl -sS -o /dev/stdout -w '\nHTTP %{http_code}\n' -X POST "${BASE}/query" \
  -H "Content-Type: application/json" \
  -d '{"query": "SELEC * FROM docs"}' || true

cat <<'EOF'

Reading the responses
---------------------
  /v1/query answers with:
    {"results": [...], "timing_ms": ..., "took_ms": ..., "rows_returned": N,
     "meta": {"velesql_contract_version": "...", "count": N}}

  - `results` rows contain exactly the projected columns. `SELECT *` returns the
    whole payload; `SELECT id, title, year` returns those three.
  - `WHERE vector NEAR $v` is the vector-search clause; the parameter is the
    query embedding, passed as a plain JSON array under "params".
  - `/v1/query/explain` answers with `query_type`, `collection`, a `plan` array,
    `estimated_cost` and `features` — and runs nothing.
  - A malformed statement is a 400 whose body carries the parse error; a
    statement pointed at a missing collection is a 404 with a VELES code.

The full statement matrix and the VelesQL grammar are in
docs/guides/MULTIMODEL_QUERIES.md and docs/reference/api-reference.md.
EOF
