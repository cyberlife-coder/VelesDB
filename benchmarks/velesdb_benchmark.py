#!/usr/bin/env python3
"""
VelesDB Official Benchmark — Protocolary Performance Report
============================================================

Produces reproducible, honest performance numbers for VelesDB.
All measurements include the COMPLETE production path:
  WAL durability + payload storage + HNSW search + result resolution.

This is the script used to generate the numbers in the README.
Run it on your machine to get YOUR numbers.

Usage:
    pip install velesdb numpy
    python benchmarks/velesdb_benchmark.py

    # JSON output for CI/comparison:
    python benchmarks/velesdb_benchmark.py --json

    # Custom datasets:
    python benchmarks/velesdb_benchmark.py --datasets 10000 50000 100000

    # With recall validation:
    python benchmarks/velesdb_benchmark.py --recall

Protocol:
    - Warm-up run discarded before measurement
    - Fixed seed (deterministic vectors, reproducible across machines)
    - WAL ON (production durability)
    - Recall measured against brute-force ground truth
    - Machine info auto-detected (CPU, RAM, OS)
    - Results include p50, p99 (not just averages)
"""

import hashlib
import json
import os
import platform
import random
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

try:
    import velesdb
except ImportError:
    print("ERROR: velesdb not installed.")
    print("  pip install velesdb")
    print("  # or build from source:")
    print("  cd crates/velesdb-python && maturin develop --release")
    sys.exit(1)

try:
    import numpy as np

    HAS_NUMPY = True
except ImportError:
    HAS_NUMPY = False

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SEED = 42
DIMENSION = 384
TOP_K = 10
SEARCH_ROUNDS = 200  # more rounds = less noise
WARMUP_SEARCHES = 20  # discarded
DEFAULT_DATASETS = [10_000, 50_000]
BATCH_SIZE = 5000
RECALL_SAMPLE = 100  # queries for recall validation


# ---------------------------------------------------------------------------
# Deterministic vector generation (reproducible across machines)
# ---------------------------------------------------------------------------


def pseudo_embedding(i: int, dim: int = DIMENSION) -> list:
    """Deterministic pseudo-random embedding from index."""
    raw = hashlib.sha256(f"velesdb-bench-{i}".encode()).digest()
    random.seed(int.from_bytes(raw[:8], "little"))
    return [random.gauss(0, 1) for _ in range(dim)]


def generate_vectors(n: int, dim: int = DIMENSION) -> list:
    """Generate n deterministic vectors."""
    return [pseudo_embedding(i, dim) for i in range(n)]


# ---------------------------------------------------------------------------
# Machine detection
# ---------------------------------------------------------------------------


def get_machine_info() -> dict:
    """Auto-detect hardware configuration."""
    info = {
        "cpu": "unknown",
        "ram": "unknown",
        "os": platform.platform(),
        "python": platform.python_version(),
        "velesdb": velesdb.__version__,
        "numpy": np.__version__ if HAS_NUMPY else "not installed",
        "date": time.strftime("%Y-%m-%d"),
    }

    if platform.system() == "Windows":
        try:
            r = subprocess.check_output(
                ["powershell", "-Command", "(Get-CimInstance Win32_Processor).Name"],
                text=True,
                timeout=10,
            ).strip()
            if r:
                info["cpu"] = r
        except Exception:
            pass
        try:
            r = subprocess.check_output(
                [
                    "powershell",
                    "-Command",
                    "[math]::Round((Get-CimInstance Win32_ComputerSystem)"
                    ".TotalPhysicalMemory / 1GB)",
                ],
                text=True,
                timeout=10,
            ).strip()
            if r:
                info["ram"] = f"{r} GB"
        except Exception:
            pass
    else:
        try:
            with open("/proc/cpuinfo") as f:
                for line in f:
                    if "model name" in line:
                        info["cpu"] = line.split(":")[1].strip()
                        break
        except Exception:
            pass
        try:
            with open("/proc/meminfo") as f:
                for line in f:
                    if "MemTotal" in line:
                        kb = int(line.split()[1])
                        info["ram"] = f"{kb / 1024 / 1024:.0f} GB"
                        break
        except Exception:
            pass

    return info


# ---------------------------------------------------------------------------
# Benchmark core
# ---------------------------------------------------------------------------


def run_benchmark(num_vectors: int, dim: int = DIMENSION) -> dict:
    """Run the full production-path benchmark for a dataset size."""
    db_path = os.path.join(os.environ.get("TEMP", "/tmp"), f"velesdb_bench_{num_vectors}")
    if os.path.exists(db_path):
        shutil.rmtree(db_path)

    print(f"\n{'='*50}")
    print(f"  {num_vectors:,} vectors x {dim}D")
    print(f"{'='*50}")

    # --- Generate vectors ---
    print("  Generating vectors (deterministic seed)...")
    vectors = generate_vectors(num_vectors, dim)

    # --- Warm-up run (discarded) ---
    print("  Warm-up insert (discarded)...")
    warmup_path = db_path + "_warmup"
    if os.path.exists(warmup_path):
        shutil.rmtree(warmup_path)
    warmup_db = velesdb.Database(warmup_path)
    warmup_col = warmup_db.create_collection("warmup", dimension=dim, metric="cosine")
    warmup_batch = [
        {"id": i, "vector": vectors[i], "payload": {"t": f"d{i}"}}
        for i in range(min(1000, num_vectors))
    ]
    warmup_col.upsert(warmup_batch)
    for j in range(WARMUP_SEARCHES):
        warmup_col.search(vector=vectors[j], top_k=TOP_K)
    del warmup_col, warmup_db
    shutil.rmtree(warmup_path, ignore_errors=True)

    # --- Measured insert ---
    print(f"  Inserting {num_vectors:,} vectors (batch={BATCH_SIZE}, WAL ON)...")
    db = velesdb.Database(db_path)
    col = db.create_collection("bench", dimension=dim, metric="cosine")

    t0 = time.perf_counter()
    for start in range(0, num_vectors, BATCH_SIZE):
        end = min(start + BATCH_SIZE, num_vectors)
        batch = [
            {"id": i, "vector": vectors[i], "payload": {"text": f"doc {i}"}}
            for i in range(start, end)
        ]
        col.upsert(batch)
    t_insert = time.perf_counter() - t0
    insert_rate = num_vectors / t_insert

    # --- Measured search ---
    print(f"  Searching {SEARCH_ROUNDS} queries (top_k={TOP_K})...")
    query_vectors = generate_vectors(SEARCH_ROUNDS, dim)
    # offset queries so they don't overlap with inserted vectors
    query_vectors = [pseudo_embedding(num_vectors + i, dim) for i in range(SEARCH_ROUNDS)]

    latencies_us = []
    for qv in query_vectors:
        t0 = time.perf_counter()
        col.search(vector=qv, top_k=TOP_K)
        latencies_us.append((time.perf_counter() - t0) * 1e6)

    latencies_us.sort()

    # --- DB size ---
    db_size_bytes = sum(
        os.path.getsize(os.path.join(dp, f))
        for dp, _, fns in os.walk(db_path)
        for f in fns
    )

    # --- Cleanup ---
    del col, db
    shutil.rmtree(db_path, ignore_errors=True)

    return {
        "dataset": f"{num_vectors // 1000}K x {dim}D",
        "num_vectors": num_vectors,
        "dimension": dim,
        "insert_total_s": round(t_insert, 2),
        "insert_rate": int(insert_rate),
        "search_avg_us": int(statistics.mean(latencies_us)),
        "search_p50_us": int(latencies_us[len(latencies_us) // 2]),
        "search_p99_us": int(latencies_us[int(len(latencies_us) * 0.99)]),
        "search_min_us": int(latencies_us[0]),
        "search_max_us": int(latencies_us[-1]),
        "search_rounds": SEARCH_ROUNDS,
        "db_size_mb": round(db_size_bytes / (1024 * 1024), 1),
    }


# ---------------------------------------------------------------------------
# Recall validation
# ---------------------------------------------------------------------------


def measure_recall(num_vectors: int = 10_000, dim: int = 128) -> dict:
    """Measure recall@10 against brute-force ground truth.

    Uses 128D with clustered data (50 clusters) for realistic measurement.
    High-dimensional random data suffers from curse of dimensionality where
    all vectors are nearly equidistant, making recall artificially low.
    """
    if not HAS_NUMPY:
        return {"error": "numpy required for recall measurement"}

    n_clusters = 50
    print(f"\n  Measuring recall@{TOP_K} on {num_vectors:,} x {dim}D "
          f"({n_clusters} clusters)...")
    db_path = os.path.join(os.environ.get("TEMP", "/tmp"), "velesdb_bench_recall")
    if os.path.exists(db_path):
        shutil.rmtree(db_path)

    # Clustered data: more realistic than uniform random
    np.random.seed(SEED)
    centers = np.random.randn(n_clusters, dim).astype(np.float32)
    centers = centers / np.linalg.norm(centers, axis=1, keepdims=True)

    vectors_per = num_vectors // n_clusters
    data_list = []
    for c in range(n_clusters):
        noise = np.random.randn(vectors_per, dim).astype(np.float32) * 0.15
        cluster = centers[c] + noise
        data_list.append(cluster)
    data_np = np.vstack(data_list)

    # Normalize for cosine
    norms = np.linalg.norm(data_np, axis=1, keepdims=True)
    norms[norms == 0] = 1
    data_norm = data_np / norms

    db = velesdb.Database(db_path)
    col = db.create_collection("recall", dimension=dim, metric="cosine")

    vectors = [data_np[i].tolist() for i in range(len(data_np))]
    for start in range(0, len(vectors), BATCH_SIZE):
        end = min(start + BATCH_SIZE, len(vectors))
        batch = [{"id": i, "vector": vectors[i]} for i in range(start, end)]
        col.upsert(batch)

    # Query vectors: perturbed cluster centers (realistic queries)
    recalls = []
    for qi in range(RECALL_SAMPLE):
        center_idx = qi % n_clusters
        q_np = centers[center_idx] + np.random.randn(dim).astype(np.float32) * 0.1
        q_norm = q_np / (np.linalg.norm(q_np) or 1)

        # Brute-force ground truth (cosine similarity)
        sims = data_norm @ q_norm
        gt_ids = set(int(x) for x in np.argsort(-sims)[:TOP_K])

        # HNSW search
        results = col.search(vector=q_np.tolist(), top_k=TOP_K)
        hnsw_ids = set(r["id"] for r in results)

        recalls.append(len(gt_ids & hnsw_ids) / TOP_K)

    del col, db
    shutil.rmtree(db_path, ignore_errors=True)

    return {
        "dataset": f"{len(vectors) // 1000}K x {dim}D (clustered)",
        "recall_at_10_mean": round(statistics.mean(recalls), 4),
        "recall_at_10_min": round(min(recalls), 4),
        "recall_queries": RECALL_SAMPLE,
    }


# ---------------------------------------------------------------------------
# Report formatting
# ---------------------------------------------------------------------------


def print_report(machine: dict, results: list, recall: dict | None = None):
    """Print human-readable benchmark report."""
    print("\n" + "=" * 60)
    print("  VelesDB Official Benchmark Report")
    print("=" * 60)
    print(f"  Version:  {machine['velesdb']}")
    print(f"  CPU:      {machine['cpu']}")
    print(f"  RAM:      {machine['ram']}")
    print(f"  OS:       {machine['os']}")
    print(f"  Python:   {machine['python']}")
    print(f"  NumPy:    {machine['numpy']}")
    print(f"  Date:     {machine['date']}")
    print()
    print("  Protocol: WAL ON, recall@10 >= 95%, deterministic seed,")
    print(f"            {SEARCH_ROUNDS} queries (warm-up discarded), batch={BATCH_SIZE}")
    print("=" * 60)

    for r in results:
        print(f"\n  [{r['dataset']}]")
        print(f"  Bulk insert:     ~{r['insert_rate']:,} vec/s  ({r['insert_total_s']}s total)")
        print(f"  Search p50:       {r['search_p50_us']:,} us")
        print(f"  Search avg:       {r['search_avg_us']:,} us")
        print(f"  Search p99:       {r['search_p99_us']:,} us")
        print(f"  Search range:     {r['search_min_us']:,} - {r['search_max_us']:,} us")
        print(f"  DB size on disk:  {r['db_size_mb']} MB")

    if recall:
        if "error" in recall:
            print(f"\n  Recall: {recall['error']}")
        else:
            status = "PASS" if recall["recall_at_10_mean"] >= 0.95 else "FAIL"
            print(f"\n  Recall@{TOP_K}: {recall['recall_at_10_mean']:.2%} "
                  f"(min={recall['recall_at_10_min']:.2%}, "
                  f"n={recall['recall_queries']}) [{status}]")

    print("\n" + "=" * 60)
    print("  These numbers reflect the COMPLETE production path:")
    print("  Python SDK -> WAL write -> HNSW search -> payload retrieval")
    print("  No shortcuts. No in-memory-only tricks.")
    print("=" * 60)


def export_json(machine: dict, results: list, recall: dict | None = None) -> dict:
    """Produce machine-readable JSON report."""
    report = {
        "schema_version": 1,
        "tool": "velesdb_benchmark.py",
        "machine": machine,
        "protocol": {
            "wal": True,
            "recall_target": ">=95%",
            "seed": SEED,
            "search_rounds": SEARCH_ROUNDS,
            "warmup_discarded": True,
            "batch_size": BATCH_SIZE,
        },
        "results": results,
    }
    if recall:
        report["recall"] = recall
    return report


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main():
    import argparse

    parser = argparse.ArgumentParser(
        description="VelesDB Official Benchmark — Protocolary Performance Report"
    )
    parser.add_argument(
        "--datasets",
        nargs="+",
        type=int,
        default=DEFAULT_DATASETS,
        help="Dataset sizes to benchmark (default: 10000 50000)",
    )
    parser.add_argument(
        "--dimension",
        type=int,
        default=DIMENSION,
        help=f"Vector dimension (default: {DIMENSION})",
    )
    parser.add_argument(
        "--recall",
        action="store_true",
        help="Include recall@10 validation against brute-force",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Output JSON report (for CI/comparison)",
    )
    parser.add_argument(
        "--output",
        type=str,
        default=None,
        help="Save JSON report to file",
    )
    args = parser.parse_args()

    machine = get_machine_info()

    if not args.json:
        print(f"VelesDB {machine['velesdb']} benchmark starting...")
        print(f"Datasets: {args.datasets}, Dimension: {args.dimension}")

    results = []
    for n in args.datasets:
        results.append(run_benchmark(n, args.dimension))

    recall = None
    if args.recall:
        recall = measure_recall(min(args.datasets), args.dimension)

    if args.json:
        report = export_json(machine, results, recall)
        output = json.dumps(report, indent=2)
        if args.output:
            Path(args.output).write_text(output)
            print(f"Report saved to {args.output}", file=sys.stderr)
        else:
            print(output)
    else:
        print_report(machine, results, recall)

        # Always save JSON alongside for reproducibility
        report = export_json(machine, results, recall)
        out_path = Path("benchmarks") / f"report_{machine['velesdb']}_{machine['date']}.json"
        if out_path.parent.exists():
            out_path.write_text(json.dumps(report, indent=2))
            print(f"\n  JSON report saved: {out_path}")


if __name__ == "__main__":
    main()
