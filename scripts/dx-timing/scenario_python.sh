#!/usr/bin/env bash
# Scenario A — Python pip install + first search.
# Path measured: "developer is in a Python container, runs pip install, runs hello-world".
# Output: single line "PYTHON_PIP <seconds>" on stdout.

set -euo pipefail

START=$(date +%s.%N)

python3 -m venv /tmp/venv
# shellcheck disable=SC1091
source /tmp/venv/bin/activate

# numpy was not declared as a runtime dependency in the velesdb wheel up to
# and including v1.13.7 (the PyO3 bindings call the NumPy C API capsule at
# first import). v1.13.8 makes numpy a hard runtime dependency, so a plain
# `pip install velesdb` is now sufficient. We keep numpy explicit here to
# stay deterministic across pre-1.13.8 wheels and to preserve the
# measurement against historical release tags. See timing-results.md
# "Honesty notes" #1.
pip install --quiet --no-cache-dir velesdb numpy

python3 - <<'PY'
import shutil, tempfile, velesdb

# velesdb.Database takes a filesystem path (no in-memory mode in the Python wrapper today).
data_dir = tempfile.mkdtemp(prefix="velesdb_dx_")
try:
    db = velesdb.Database(data_dir)
    col = db.create_collection("hello", 4)
    col.upsert(1, [0.1, 0.2, 0.3, 0.4], payload={"name": "alpha"})
    col.upsert(2, [0.5, 0.6, 0.7, 0.8], payload={"name": "beta"})
    results = col.search([0.1, 0.2, 0.3, 0.4], top_k=2)
    assert len(results) == 2, f"expected 2 results, got {len(results)}"
    # Results are list[dict] with keys: id, score, plus payload fields flattened.
    print(f"first match: id={results[0]['id']} score={results[0]['score']:.4f}")
finally:
    shutil.rmtree(data_dir, ignore_errors=True)
PY

END=$(date +%s.%N)
ELAPSED=$(awk "BEGIN {printf \"%.2f\", $END - $START}")
echo "PYTHON_PIP $ELAPSED"
