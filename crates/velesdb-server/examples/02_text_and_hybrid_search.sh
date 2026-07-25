#!/usr/bin/env bash
# 02_text_and_hybrid_search.sh — the three retrieval modes on one corpus.
#
#   dense  : POST /v1/collections/{name}/search          {"vector", "top_k"}
#   text   : POST /v1/collections/{name}/search/text     {"query",  "top_k"}
#   hybrid : POST /v1/collections/{name}/search/hybrid   {"vector", "query", "top_k", "vector_weight"}
#
# BM25 text search reads the string fields of each point's payload; no separate
# index build step is required. Hybrid fuses the two rankings, weighted by
# `vector_weight` (0.0 = text only, 1.0 = vector only, default 0.5).
#
# Usage:
#   ./02_text_and_hybrid_search.sh
#   VELESDB_SERVER_BIN=/path/to/velesdb-server ./02_text_and_hybrid_search.sh

set -euo pipefail

BIN="${VELESDB_SERVER_BIN:-velesdb-server}"
PORT="${PORT:-8082}"
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

echo "==> Create the collection"
curl -sS -X POST "${BASE}/collections" \
  -H "Content-Type: application/json" \
  -d '{"name": "articles", "dimension": 4, "metric": "cosine"}'
echo

echo
echo "==> Insert four articles (vector + text payload)"
curl -sS -X POST "${BASE}/collections/articles/points" \
  -H "Content-Type: application/json" \
  -d '{"points": [
        {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0],
         "payload": {"title": "Rust ownership explained", "body": "borrow checker lifetimes rust", "lang": "en"}},
        {"id": 2, "vector": [0.9, 0.3, 0.0, 0.0],
         "payload": {"title": "Async Rust in practice", "body": "futures executors rust tokio", "lang": "en"}},
        {"id": 3, "vector": [0.0, 1.0, 0.0, 0.0],
         "payload": {"title": "Postgres index tuning", "body": "btree gin vacuum planner", "lang": "en"}},
        {"id": 4, "vector": [0.0, 0.9, 0.3, 0.0],
         "payload": {"title": "Sharding relational data", "body": "partition routing planner", "lang": "en"}}
      ]}'
echo

echo
echo "==> Dense vector search (top 2 near the first axis)"
curl -sS -X POST "${BASE}/collections/articles/search" \
  -H "Content-Type: application/json" \
  -d '{"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 2}'
echo

echo
echo "==> BM25 text search for \"rust\""
curl -sS -X POST "${BASE}/collections/articles/search/text" \
  -H "Content-Type: application/json" \
  -d '{"query": "rust", "top_k": 3}'
echo

echo
echo "==> Hybrid search: same query vector plus the word \"planner\", 30% vector / 70% text"
curl -sS -X POST "${BASE}/collections/articles/search/hybrid" \
  -H "Content-Type: application/json" \
  -d '{"vector": [1.0, 0.0, 0.0, 0.0], "query": "planner", "top_k": 3, "vector_weight": 0.3}'
echo

echo
echo "==> Dense search narrowed by a payload filter (lang = en, still top 2)"
curl -sS -X POST "${BASE}/collections/articles/search" \
  -H "Content-Type: application/json" \
  -d '{"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 2,
       "filter": {"condition": {"type": "eq", "field": "lang", "value": "en"}}}'
echo

cat <<'EOF'

Reading the three rankings
--------------------------
  - dense search ranks by cosine similarity only: ids 1 and 2 win because their
    vectors point along the first axis;
  - text search ranks by BM25 over the payload strings: the two articles whose
    body contains "rust" come back, the database ones do not;
  - hybrid mixes both. Lowering vector_weight moves the ranking towards the
    text side, which is what lets id 3 and id 4 ("planner") surface even though
    their vectors are orthogonal to the query.

Scores are not comparable across modes: cosine similarity, BM25 and the fused
score live on different scales. Compare ranks, not numbers.
EOF
