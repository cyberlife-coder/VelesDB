"""
VelesDB Bulk Insert Performance Benchmark
==========================================

Measures insert throughput with the optimized upsert_bulk path
and compares against published market benchmarks.

Usage:
    cd crates/velesdb-python && maturin develop --release
    cd ../.. && python benchmarks/benchmark_bulk_insert.py

Market reference points (HNSW, single-node, 768D, cosine):
    Qdrant v1.12    ~15-25K vec/s  (Rust, gRPC, with WAL)
    Weaviate v1.28  ~8-12K vec/s   (Go, REST, with WAL)
    Milvus v2.5     ~20-40K vec/s  (Go/C++, gRPC, with WAL)
    ChromaDB v0.6   ~3-5K vec/s    (Python/Rust, in-process)
    pgvector 0.8    ~1-3K vec/s    (PostgreSQL extension)
    LanceDB v0.15   ~10-20K vec/s  (Rust, in-process, no HNSW)

Note: Market numbers vary widely based on hardware, batch size, durability
settings, and whether indexing is deferred. These are rough reference points
from public benchmarks (ann-benchmarks, VectorDBBench, vendor docs).
VelesDB is in-process (no network overhead) with full HNSW + WAL + fsync.
"""

import json
import os
import platform
import shutil
import sys
import time
from pathlib import Path

import numpy as np

try:
    import velesdb
except ImportError:
    print("velesdb Python package not found.")
    print("Build it first: cd crates/velesdb-python && maturin develop --release")
    sys.exit(1)


def generate_vectors(count: int, dim: int, seed: int = 42) -> np.ndarray:
    """Generate normalized random vectors."""
    rng = np.random.default_rng(seed)
    vectors = rng.standard_normal((count, dim)).astype(np.float32)
    norms = np.linalg.norm(vectors, axis=1, keepdims=True)
    norms[norms == 0] = 1.0
    return vectors / norms


def measure_bulk_insert(
    db_path: str, vectors: np.ndarray, batch_size: int, use_numpy: bool = False
):
    """Measure bulk insert throughput and return metrics."""
    dim = vectors.shape[1]
    count = vectors.shape[0]

    # Fresh database
    if os.path.exists(db_path):
        shutil.rmtree(db_path)

    db = velesdb.Database(db_path)
    db.create_collection("bench", dim, "cosine")
    col = db.get_collection("bench")

    # Measure batch insert
    batch_times = []
    total_start = time.perf_counter()

    if use_numpy:
        # Fast path: numpy arrays directly
        ids = np.arange(count, dtype=np.uint64)
        for start in range(0, count, batch_size):
            end = min(start + batch_size, count)
            batch_start = time.perf_counter()
            col.upsert_bulk_numpy(
                vectors[start:end],
                ids[start:end].tolist(),
            )
            batch_elapsed = time.perf_counter() - batch_start
            batch_times.append(batch_elapsed)
    else:
        # Dict path: Python dicts
        points = [
            {"id": i, "vector": vectors[i].tolist()}
            for i in range(count)
        ]
        for start in range(0, count, batch_size):
            end = min(start + batch_size, count)
            batch = points[start:end]
            batch_start = time.perf_counter()
            col.upsert_bulk(batch)
            batch_elapsed = time.perf_counter() - batch_start
            batch_times.append(batch_elapsed)

    total_elapsed = time.perf_counter() - total_start

    # Compute metrics
    throughput = count / total_elapsed

    # Disk size
    db_size = sum(
        f.stat().st_size
        for f in Path(db_path).rglob("*")
        if f.is_file()
    )

    # Per-batch percentiles
    batch_times_ms = [t * 1000 for t in batch_times]
    p50 = float(np.percentile(batch_times_ms, 50))
    p95 = float(np.percentile(batch_times_ms, 95))
    p99 = float(np.percentile(batch_times_ms, 99))

    del db

    return {
        "count": count,
        "dimension": dim,
        "batch_size": batch_size,
        "total_time_s": round(total_elapsed, 3),
        "throughput_vec_s": round(throughput, 1),
        "batch_latency_p50_ms": round(p50, 1),
        "batch_latency_p95_ms": round(p95, 1),
        "batch_latency_p99_ms": round(p99, 1),
        "disk_size_mb": round(db_size / (1024 * 1024), 1),
    }


def run_benchmark():
    """Run the full benchmark suite."""
    print("=" * 70)
    print("VelesDB Bulk Insert Benchmark")
    print("=" * 70)

    # System info
    import cpuinfo
    try:
        cpu = cpuinfo.get_cpu_info()["brand_raw"]
    except Exception:
        cpu = platform.processor() or "Unknown"

    import psutil
    ram_gb = round(psutil.virtual_memory().total / (1024**3))

    print(f"CPU:      {cpu}")
    print(f"RAM:      {ram_gb} GB")
    print(f"OS:       {platform.system()} {platform.version()}")
    print(f"Python:   {platform.python_version()}")
    print(f"Date:     {time.strftime('%Y-%m-%d')}")
    print()

    configs = [
        # (count, dimension, batch_size)
        (10_000, 384, 1_000),
        (50_000, 384, 5_000),
        (10_000, 768, 1_000),
        (50_000, 768, 5_000),
    ]

    results = []
    db_path = os.path.join(os.environ.get("TEMP", "/tmp"), "velesdb_bench")

    # --- Dict path (baseline) ---
    print("  --- Dict path (upsert_bulk) ---")
    print()
    for count, dim, batch_size in configs:
        print(f"  [{count:,} vectors, {dim}D, batch={batch_size}]")
        vectors = generate_vectors(count, dim)

        result = measure_bulk_insert(db_path, vectors, batch_size, use_numpy=False)
        result["path"] = "dict"
        results.append(result)

        print(f"    Throughput:     {result['throughput_vec_s']:,.0f} vec/s")
        print(f"    Total time:    {result['total_time_s']:.2f}s")
        print()

    # --- Numpy path (optimized) ---
    print("  --- Numpy path (upsert_bulk_numpy) ---")
    print()
    for count, dim, batch_size in configs:
        print(f"  [{count:,} vectors, {dim}D, batch={batch_size}]")
        vectors = generate_vectors(count, dim)

        result = measure_bulk_insert(db_path, vectors, batch_size, use_numpy=True)
        result["path"] = "numpy"
        results.append(result)

        print(f"    Throughput:     {result['throughput_vec_s']:,.0f} vec/s")
        print(f"    Total time:    {result['total_time_s']:.2f}s")
        print(f"    Batch p50:     {result['batch_latency_p50_ms']:.1f} ms")
        print(f"    Batch p99:     {result['batch_latency_p99_ms']:.1f} ms")
        print(f"    Disk size:     {result['disk_size_mb']:.1f} MB")
        print()

    # Cleanup
    if os.path.exists(db_path):
        shutil.rmtree(db_path)

    # Market comparison
    print("=" * 70)
    print("Market Comparison (HNSW, single-node, cosine)")
    print("=" * 70)
    print()

    # Find 50K/768D result for comparison
    ref = next((r for r in results if r["count"] == 50_000 and r["dimension"] == 768), None)
    if ref is None:
        ref = next((r for r in results if r["count"] == 50_000), results[-1])

    ref_throughput = ref["throughput_vec_s"]

    market = [
        ("Milvus v2.5",     "20-40K", 30000),
        ("Qdrant v1.12",    "15-25K", 20000),
        ("LanceDB v0.15",   "10-20K", 15000),
        ("Weaviate v1.28",  "8-12K",  10000),
        ("ChromaDB v0.6",   "3-5K",   4000),
        ("pgvector 0.8",    "1-3K",   2000),
    ]

    print(f"  {'Engine':<20} {'Published':<12} {'VelesDB ratio':<15} {'Notes'}")
    print(f"  {'-'*20} {'-'*12} {'-'*15} {'-'*30}")

    for name, published, midpoint in market:
        ratio = ref_throughput / midpoint
        bar = "#" * min(int(ratio * 10), 30)
        print(f"  {name:<20} {published:<12} {ratio:.2f}x          {bar}")

    print()
    print(f"  VelesDB (this run): {ref_throughput:,.0f} vec/s "
          f"({ref['count']:,} x {ref['dimension']}D, batch={ref['batch_size']})")
    print()
    print("  Note: VelesDB is in-process (no network). Server engines (Qdrant,")
    print("  Milvus, Weaviate) include gRPC/REST overhead. Fair comparison is")
    print("  with ChromaDB/LanceDB (also in-process) or via velesdb-server REST.")

    # Save results
    output = {
        "engine": "VelesDB",
        "version": "1.7.0",
        "cpu": cpu,
        "ram_gb": ram_gb,
        "os": f"{platform.system()} {platform.version()}",
        "date": time.strftime("%Y-%m-%d"),
        "results": results,
    }

    output_path = os.path.join(
        os.path.dirname(__file__), "results", "bulk_insert_latest.json"
    )
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(output, f, indent=2)
    print(f"  Results saved to: {output_path}")
    print()


if __name__ == "__main__":
    run_benchmark()
