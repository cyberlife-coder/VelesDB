#!/usr/bin/env bash
# VelesDB CLI demo — the whole engine in under 3 minutes.
#
#   seed -> ANN + metadata filter -> graph traversal -> columnstore GROUP BY
#        -> hybrid fusion (rrf_k, beating each branch alone)
#        -> EXPLAIN -> REST live -> export
#
# Deterministic: fixed data, no network access. Idempotent: the work dir is
# recreated on every run. Pretty mode is on by default (tables + ANSI color);
# NO_COLOR or --ci disables it. `--check` re-runs headless and asserts the
# transcript's shape so CI can smoke it.
#
# Usage: demos/cli-demo.sh [--db PATH] [--keep] [--check] [--ci]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="${TMPDIR:-/tmp}/velesdb-cli-demo"
OUT="${TMPDIR:-/tmp}/velesdb-cli-demo.out"
KEEP=0 CHECK=0 CI=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --db)    DB="$2"; shift 2 ;;
        --keep)  KEEP=1; shift ;;
        --check) CHECK=1; shift ;;
        --ci)    CI=1; shift ;;
        *)       shift ;;
    esac
done
[[ $CI -eq 1 ]] && export NO_COLOR=1
export VELESDB_NO_UPDATE_CHECK=1   # deterministic, offline

VELESDB="${VELESDB:-$ROOT/target/debug/velesdb}"
SERVER="${VELESDB_SERVER:-$ROOT/target/debug/velesdb-server}"

if [[ ! -x "$VELESDB" ]]; then
    echo "velesdb binary not found at $VELESDB" >&2
    echo "Build it: cargo build -p velesdb-cli" >&2
    exit 1
fi

# --check: run the demo headless, assert the transcript shape. No exact-score
# pinning (HNSW build RNG may reshuffle close scores); the fusion claim is
# structural: doc 2 tops the fused list while missing from each branch's top.
if [[ $CHECK -eq 1 ]]; then
    bash "$0" --db "$DB" --keep --ci >"$OUT" 2>&1 || { cat "$OUT"; echo "demo run failed" >&2; exit 1; }
    failures=0
    expect() {
        if grep -q "$2" "$OUT"; then
            printf '  ok   %s\n' "$1"
        else
            printf '  FAIL %s (missing %s)\n' "$1" "$2" >&2
            failures=$((failures + 1))
        fi
    }
    expect "seed"            "created (schemaless, embeddings 4D)"
    expect "ANN+filter"      "LSM-tree internals"
    expect "graph"           "WROTE"
    expect "columnstore"     "GROUP BY"
    expect "branch vector"   "SIMD optimization"
    expect "branch text"     "Rust cookbook"
    expect "fused top hit"   "Rust programming guide"
    expect "explain"         "FUSION"
    expect "export"          "Exporting"
    expect "under 3 min"     "done in [0-9]"
    if [[ $failures -eq 0 ]]; then
        echo "cli-demo --check: PASS ($(grep -c . "$OUT") transcript lines in $OUT)"
    else
        exit 1
    fi
    rm -rf "$DB" "$OUT"
    exit 0
fi

# ---------------------------------------------------------------------------
# Pretty printing
# ---------------------------------------------------------------------------
if [[ -n "${NO_COLOR:-}" ]]; then
    B="" D="" G="" C="" Y=""
else
    B=$'\033[1m' D=$'\033[2m' G=$'\033[32m' C=$'\033[36m' Y=$'\033[33m'
fi
T0=$SECONDS
step() { printf "\n${B}${C}── %s ${D}(%ds)\n" "$1" "$((SECONDS - T0))"; }
note() { printf "${D}%s${D}\n" "$1"; }
hr()   { printf "${D}%s${D}\n" "────────────────────────────────────────────"; }
run()  { printf "${Y}\$${D} %s\n" "$*"; "$@"; }

SRV_PID=""
cleanup() {
    [[ -n "$SRV_PID" ]] && kill "$SRV_PID" 2>/dev/null
    [[ $KEEP -eq 0 ]] && rm -rf "$DB"
    return 0
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# 0. Seed — deterministic, no network
# ---------------------------------------------------------------------------
printf "${B}${G}VelesDB CLI demo${D} — vector + graph + columnstore + hybrid fusion, one binary.\n"
rm -rf "$DB"; mkdir -p "$DB"

step "seed: 6 documents, fixed vectors (no network, no model)"
run "$VELESDB" collection create "$DB" docs -d 4 -m cosine
for row in \
  '1|[0.90,0.10,0.00,0.00]|{"title":"LSM-tree internals","content":"storage engine design","category":"tech","price":30}' \
  '2|[0.70,0.10,0.10,0.00]|{"title":"Rust programming guide","content":"rust borrow checker and ownership","category":"tech","price":45}' \
  '3|[0.00,0.00,0.90,0.10]|{"title":"Rust cookbook","content":"rust recipes for systems programmers","category":"food","price":20}' \
  '4|[0.85,0.00,0.00,0.15]|{"title":"SIMD optimization","content":"avx512 vector kernels","category":"tech","price":60}' \
  '5|[0.10,0.90,0.00,0.00]|{"title":"Graph traversal algorithms","content":"bfs dfs shortest path","category":"tech","price":35}' \
  '6|[0.00,0.00,0.00,1.00]|{"title":"Banana bread","content":"easy baking recipe","category":"food","price":10}'
do
    IFS='|' read -r id vec payload <<<"$row"
    "$VELESDB" data upsert "$DB" docs --id "$id" --vector "$vec" --payload "$payload" >/dev/null
done
note "6 documents upserted into 'docs' (deterministic vectors — distances below are stable)."

# ---------------------------------------------------------------------------
# 1. ANN + metadata filter
# ---------------------------------------------------------------------------
step "ANN search + metadata filter: near [1,0,0,0] AND category='tech'"
run "$VELESDB" query execute "$DB" \
    "SELECT id, title, category FROM docs WHERE vector NEAR [1.0, 0.0, 0.0, 0.0] AND category = 'tech' LIMIT 5"

# ---------------------------------------------------------------------------
# 2. Graph: a small knowledge graph over the same corpus
# ---------------------------------------------------------------------------
step "graph: authors -> documents (schemaless collection, node embeddings)"
run "$VELESDB" collection create-graph "$DB" kg -d 4
run "$VELESDB" graph store-payload "$DB" kg 100 '{"name":"Ada","kind":"author"}'   --vector '[0.9,0.1,0.0,0.0]'
run "$VELESDB" graph store-payload "$DB" kg 200 '{"name":"Grace","kind":"author"}' --vector '[0.0,0.0,0.9,0.1]'
run "$VELESDB" graph store-payload "$DB" kg 1 '{"name":"LSM-tree internals","kind":"doc"}' --vector '[0.90,0.10,0.00,0.00]'
run "$VELESDB" graph store-payload "$DB" kg 3 '{"name":"Rust cookbook","kind":"doc"}'      --vector '[0.00,0.00,0.90,0.10]'
run "$VELESDB" graph store-payload "$DB" kg 4 '{"name":"SIMD optimization","kind":"doc"}'  --vector '[0.85,0.00,0.00,0.15]'
run "$VELESDB" graph add-edge "$DB" kg 1 100 1 "WROTE"
run "$VELESDB" graph add-edge "$DB" kg 2 100 4 "WROTE"
run "$VELESDB" graph add-edge "$DB" kg 3 200 3 "WROTE"
run "$VELESDB" graph traverse "$DB" kg 100 --algorithm bfs --max-depth 2
run "$VELESDB" graph search "$DB" kg '[0.9,0.1,0.0,0.0]' -k 3

# ---------------------------------------------------------------------------
# 3. ColumnStore aggregation
# ---------------------------------------------------------------------------
step "columnstore: GROUP BY aggregation over the same docs"
run "$VELESDB" query execute "$DB" \
    "SELECT category, COUNT(*), AVG(price) FROM docs GROUP BY category"

# ---------------------------------------------------------------------------
# 4. Hybrid fusion — rrf_k beats each branch alone, side by side
# ---------------------------------------------------------------------------
step "branch 1 alone: vector NEAR (no 'rust' filter)"
run "$VELESDB" query execute "$DB" \
    "SELECT id, title FROM docs WHERE vector NEAR [1.0, 0.0, 0.0, 0.0] LIMIT 3"
step "branch 2 alone: text MATCH 'rust'"
run "$VELESDB" query execute "$DB" \
    "SELECT id, title FROM docs WHERE content MATCH 'rust' LIMIT 3"
step "fused: NEAR + MATCH, USING FUSION rrf (k=60)"
run "$VELESDB" query execute "$DB" \
    "SELECT id, title FROM docs WHERE vector NEAR [1.0, 0.0, 0.0, 0.0] AND content MATCH 'rust' LIMIT 3 USING FUSION(strategy='rrf', k=60)"
note "Doc 2 sits mid-rank in each single branch yet tops the fused list — that is what RRF buys you."

# ---------------------------------------------------------------------------
# 5. EXPLAIN — the plan, not a promise
# ---------------------------------------------------------------------------
step "EXPLAIN: the fused query's plan"
run "$VELESDB" query explain "$DB" \
    "SELECT id, title FROM docs WHERE vector NEAR [1.0, 0.0, 0.0, 0.0] AND content MATCH 'rust' LIMIT 3 USING FUSION(strategy='rrf', k=60)"

# ---------------------------------------------------------------------------
# 6. REST live — same engine over HTTP (skipped if server binary is absent)
# ---------------------------------------------------------------------------
step "REST live: velesdb-server answers the same query over HTTP"
if [[ -x "$SERVER" ]]; then
    PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
    "$SERVER" --data-dir "$DB" --port "$PORT" >/dev/null 2>&1 &
    SRV_PID=$!
    for _ in $(seq 1 20); do curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break; sleep 0.3; done
    run curl -s "http://127.0.0.1:$PORT/health"
    echo
    run curl -s -X POST "http://127.0.0.1:$PORT/query" \
        -H 'Content-Type: application/json' \
        -d '{"query":"SELECT id, title FROM docs WHERE vector NEAR [1.0,0.0,0.0,0.0] LIMIT 2","collection":"docs"}'
    echo
    kill "$SRV_PID" 2>/dev/null
    for _ in $(seq 1 20); do kill -0 "$SRV_PID" 2>/dev/null || break; sleep 0.3; done
    SRV_PID=""
    note "same data dir the CLI just wrote — one binary, one format, embedded or served."
else
    note "velesdb-server not built — skipping (cargo build -p velesdb-server to enable)."
fi

# ---------------------------------------------------------------------------
# 7. Export — the whole collection back out as JSON
# ---------------------------------------------------------------------------
step "export: docs -> JSON"
run "$VELESDB" data export "$DB" docs --output "$DB/export.json"
head -c 400 "$DB/export.json"; echo " ..."

hr
printf "${B}${G}done${D} in %ds — install: cargo install velesdb-cli · docs: docs/guides/CLI_COMMAND_REFERENCE.md · star the repo if this was useful.\n" "$((SECONDS - T0))"
