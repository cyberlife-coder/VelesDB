#!/usr/bin/env python3
"""
Quick VelesDB Benchmark (No Dependencies)
==========================================

Runs VelesDB-only benchmark without requiring Docker/PostgreSQL.
Useful for quick performance checks.

Usage:
    python benchmark_quick.py [--vectors N] [--dim D]
"""

import argparse
import time
import os
import shutil
import numpy as np


def generate_normalized_vectors(n: int, dim: int) -> np.ndarray:
    """Generate L2-normalized random vectors."""
    print(f"Generating {n} normalized vectors (dim={dim})...")
    data = np.random.randn(n, dim).astype('float32')
    norms = np.linalg.norm(data, axis=1, keepdims=True)
    return data / norms


def run_benchmark(n_vectors: int, dim: int, n_queries: int = 100):
    """Run VelesDB benchmark."""
    try:
        import velesdb
    except ImportError:
        print("ERROR: velesdb not installed. Run: pip install velesdb")
        return

    data_dir = "./quick_bench_data"

    # Cleanup
    if os.path.exists(data_dir):
        shutil.rmtree(data_dir)

    # Generate data
    data = generate_normalized_vectors(n_vectors, dim)
    queries = generate_normalized_vectors(n_queries, dim)

    print("\n" + "=" * 50)
    print("VELESDB QUICK BENCHMARK")
    print("=" * 50)

    # Setup
    db = velesdb.Database(data_dir)
    collection = db.create_collection("vectors", dimension=dim, metric="cosine")

    # Insertion
    print(f"\nInserting {n_vectors} vectors...")
    points = [{"id": i, "vector": v.tolist()} for i, v in enumerate(data)]

    start = time.time()
    if hasattr(collection, 'upsert_bulk'):
        collection.upsert_bulk(points)
    else:
        batch_size = 1000
        for i in range(0, len(points), batch_size):
            collection.upsert(points[i:i + batch_size])
    insert_time = time.time() - start

    print(f"  Time: {insert_time:.3f}s")
    print(f"  Throughput: {n_vectors / insert_time:.0f} vectors/sec")

    # Search
    print(f"\nRunning {n_queries} search queries (top_k=10)...")

    # Warmup
    for q in queries[:10]:
        collection.search(q.tolist(), top_k=10)

    latencies = []
    for q in queries:
        start = time.time()
        collection.search(q.tolist(), top_k=10)
        latencies.append(time.time() - start)

    avg_lat = np.mean(latencies) * 1000
    p50_lat = np.percentile(latencies, 50) * 1000
    p95_lat = np.percentile(latencies, 95) * 1000
    p99_lat = np.percentile(latencies, 99) * 1000
    qps = 1.0 / np.mean(latencies)

    print(f"  Avg Latency: {avg_lat:.2f}ms")
    print(f"  P50 Latency: {p50_lat:.2f}ms")
    print(f"  P95 Latency: {p95_lat:.2f}ms")
    print(f"  P99 Latency: {p99_lat:.2f}ms")
    print(f"  Throughput: {qps:.1f} QPS")

    print("\n" + "=" * 50)
    print("SUMMARY")
    print("=" * 50)
    print(f"Dataset: {n_vectors} vectors x {dim} dimensions")
    print(f"Insert: {insert_time:.2f}s ({n_vectors / insert_time:.0f} vec/s)")
    print(f"Search: {avg_lat:.2f}ms avg, {qps:.0f} QPS")
    print("=" * 50)

    # Cleanup
    shutil.rmtree(data_dir)


def main():
    parser = argparse.ArgumentParser(description="Quick VelesDB Benchmark")
    parser.add_argument("--vectors", type=int, default=10000, help="Number of vectors")
    parser.add_argument("--dim", type=int, default=768, help="Vector dimension")
    parser.add_argument("--queries", type=int, default=100, help="Number of queries")
    args = parser.parse_args()

    run_benchmark(args.vectors, args.dim, args.queries)


if __name__ == "__main__":
    main()
