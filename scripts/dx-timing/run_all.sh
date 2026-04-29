#!/usr/bin/env bash
# Run the four DX onboarding scenarios three times each, collect timings,
# and emit a JSON report. See scripts/dx-timing/README.md for context.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
OUT_DIR="$REPO_ROOT/benchmarks/dx-timing"
mkdir -p "$OUT_DIR"
TIMESTAMP=$(date -u +%Y-%m-%dT%H-%M-%SZ)
OUT_FILE="$OUT_DIR/results-$TIMESTAMP.json"
TIMING_RUN_ID="$TIMESTAMP-$$"
RUNS_PER_SCENARIO=3
SLO_SECONDS=300

# Detect Docker socket binding for Git-Bash on Windows. Translate the
# script directory to a path Docker Desktop on Windows accepts.
DOCKER_CTX_DIR="$SCRIPT_DIR"
if command -v cygpath >/dev/null 2>&1; then
    export MSYS_NO_PATHCONV=1
    DOCKER_CTX_DIR=$(cygpath -w "$SCRIPT_DIR")
fi

build_image() {
    local name="$1" dockerfile="$2"
    docker build --quiet \
        -f "$DOCKER_CTX_DIR/$dockerfile" \
        --build-arg "TIMING_RUN_ID=$TIMING_RUN_ID" \
        -t "velesdb-dx-$name:$TIMING_RUN_ID" \
        "$DOCKER_CTX_DIR" >/dev/null
}

run_in_container() {
    # $1 = image name (without tag), $2 = scenario script path inside repo
    local img="velesdb-dx-$1:$TIMING_RUN_ID"
    local script_host_path="$DOCKER_CTX_DIR/$2"
    docker run --rm \
        -v "$script_host_path:/scenario.sh:ro" \
        "$img" bash //scenario.sh
}

# Server scenario runs on the host's Rust image (--network host so the
# probe binds to localhost). Same image as Rust scenario, reused.
run_server_scenario() {
    docker run --rm --network host \
        -v "$DOCKER_CTX_DIR/scenario_server.sh:/scenario.sh:ro" \
        "velesdb-dx-rust:$TIMING_RUN_ID" bash -c '
            apt-get update -qq && apt-get install -y --no-install-recommends curl >/dev/null 2>&1
            bash //scenario.sh
        '
}

extract_seconds() {
    # Each scenario prints its result on stdout as "<TAG> <seconds>".
    # Pull the trailing numeric token regardless of TAG.
    awk 'END { print $NF }' <<<"$1"
}

median() {
    # Three numeric inputs on three lines, sorted.
    sort -g | awk 'NR==2 { print }'
}

run_scenario() {
    local name="$1" runner="$2" script="$3"
    local results=()
    echo "── $name ──"
    for i in $(seq 1 "$RUNS_PER_SCENARIO"); do
        echo "  run $i/$RUNS_PER_SCENARIO ..."
        local out
        if [ "$runner" = "container" ]; then
            out=$(run_in_container "$name" "$script")
        else
            out=$(run_server_scenario)
        fi
        local sec
        sec=$(extract_seconds "$out")
        echo "    -> ${sec}s"
        results+=("$sec")
    done
    local med
    med=$(printf '%s\n' "${results[@]}" | median)
    SCENARIO_RESULTS+=("{\"name\":\"$name\",\"runs\":[${results[0]},${results[1]},${results[2]}],\"median\":$med}")
    echo "  median: ${med}s"

    awk -v m="$med" -v slo="$SLO_SECONDS" 'BEGIN { exit (m+0 > slo+0) ? 1 : 0 }' \
        || { echo "  SLO BUST (>$SLO_SECONDS s)"; SLO_BUSTED=1; }
}

echo "VelesDB DX onboarding timing — $TIMESTAMP"
echo "  $RUNS_PER_SCENARIO runs per scenario, SLO $SLO_SECONDS s"
echo

echo "Building Docker images (TIMING_RUN_ID=$TIMING_RUN_ID)..."
build_image python Dockerfile.python
build_image rust   Dockerfile.rust
build_image node   Dockerfile.node
echo "  done."
echo

SCENARIO_RESULTS=()
SLO_BUSTED=0

run_scenario python container scenario_python.sh
run_scenario rust   container scenario_rust.sh
run_scenario node   container scenario_node.sh
run_scenario server host       scenario_server.sh

# Emit JSON.
HOST_OS=$(uname -s 2>/dev/null || echo unknown)
HOST_ARCH=$(uname -m 2>/dev/null || echo unknown)
SCENARIOS_JSON=$(printf '%s,' "${SCENARIO_RESULTS[@]}" | sed 's/,$//')
cat > "$OUT_FILE" <<JSON
{
  "timestamp": "$TIMESTAMP",
  "timing_run_id": "$TIMING_RUN_ID",
  "host": { "os": "$HOST_OS", "arch": "$HOST_ARCH" },
  "runs_per_scenario": $RUNS_PER_SCENARIO,
  "slo_seconds": $SLO_SECONDS,
  "scenarios": [$SCENARIOS_JSON]
}
JSON

echo
echo "Results written to: $OUT_FILE"
cat "$OUT_FILE"
echo

if [ "$SLO_BUSTED" = "1" ]; then
    echo "⚠ At least one scenario exceeded the $SLO_SECONDS s SLO."
    exit 1
fi
echo "✓ All scenarios under $SLO_SECONDS s."
