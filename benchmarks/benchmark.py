#!/usr/bin/env python3
"""
VelesDB vs pgvectorscale Benchmark
==================================

Comprehensive benchmark comparing VelesDB against pgvectorscale (DiskANN).

Usage:
    python benchmark.py [--vectors N] [--dim D] [--queries Q]

Requirements:
    - Docker running with pgvectorscale container (docker-compose up -d)
    - pip install -r requirements.txt
"""

import argparse
import time
import os
import shutil
import gc
import sys
from dataclasses import dataclass
from typing import Optional

import numpy as np

# Configuration
@dataclass
class BenchConfig:
    n_vectors: int = 10000
    dim: int = 768
    n_queries: int = 100
    top_k: int = 10
    batch_size: int = 1000
    pg_url: str = "postgres://postgres:benchpass@localhost:5433/benchmark"
    veles_data_dir: str = "./bench_data"
    metric: str = "cosine"

@dataclass
class BenchResults:
    insert_time: float = 0.0
    index_time: float = 0.0
    avg_latency_ms: float = 0.0
    p50_latency_ms: float = 0.0
    p95_latency_ms: float = 0.0
    p99_latency_ms: float = 0.0
    qps: float = 0.0


def generate_normalized_vectors(n: int, dim: int) -> np.ndarray:
    """Generate L2-normalized random vectors (simulating embeddings)."""
    print(f"Generating {n} normalized vectors (dim={dim})...")
    data = np.random.randn(n, dim).astype('float32')
    norms = np.linalg.norm(data, axis=1, keepdims=True)
    return data / norms


def bench_pgvectorscale(config: BenchConfig, data: np.ndarray, queries: np.ndarray) -> Optional[BenchResults]:
    """Benchmark pgvectorscale (PostgreSQL + DiskANN)."""
    try:
        import psycopg2
    except ImportError:
        print("ERROR: psycopg2 not installed. Run: pip install psycopg2-binary")
        return None

    print("\n" + "=" * 60)
    print("PGVECTORSCALE (DiskANN)")
    print("=" * 60)

    results = BenchResults()

    try:
        conn = psycopg2.connect(config.pg_url)
        cur = conn.cursor()

        # Setup
        print("Setting up PostgreSQL...")
        cur.execute("CREATE EXTENSION IF NOT EXISTS vectorscale CASCADE;")
        cur.execute("DROP TABLE IF EXISTS bench_vectors;")
        cur.execute(f"CREATE TABLE bench_vectors (id serial PRIMARY KEY, embedding vector({config.dim}));")
        conn.commit()

        # Insertion
        print(f"Inserting {len(data)} vectors...")
        start = time.time()
        for i in range(0, len(data), config.batch_size):
            batch = data[i:i + config.batch_size]
            args_str = ','.join(
                cur.mogrify("(%s::vector)", (v.tolist(),)).decode('utf-8')
                for v in batch
            )
            cur.execute("INSERT INTO bench_vectors (embedding) VALUES " + args_str)
        conn.commit()
        results.insert_time = time.time() - start
        print(f"  Insert time: {results.insert_time:.3f}s ({len(data) / results.insert_time:.0f} vec/s)")

        # Indexing
        print("Building DiskANN index...")
        start = time.time()
        cur.execute("CREATE INDEX ON bench_vectors USING diskann (embedding vector_cosine_ops);")
        conn.commit()
        results.index_time = time.time() - start
        print(f"  Index build time: {results.index_time:.3f}s")

        # Search
        print(f"Running {len(queries)} search queries...")
        # Warmup
        for q in queries[:10]:
            cur.execute(
                "SELECT id FROM bench_vectors ORDER BY embedding <=> %s::vector LIMIT %s",
                (q.tolist(), config.top_k)
            )
            cur.fetchall()

        latencies = []
        for q in queries:
            start = time.time()
            cur.execute(
                "SELECT id FROM bench_vectors ORDER BY embedding <=> %s::vector LIMIT %s",
                (q.tolist(), config.top_k)
            )
            cur.fetchall()
            latencies.append(time.time() - start)

        results.avg_latency_ms = np.mean(latencies) * 1000
        results.p50_latency_ms = np.percentile(latencies, 50) * 1000
        results.p95_latency_ms = np.percentile(latencies, 95) * 1000
        results.p99_latency_ms = np.percentile(latencies, 99) * 1000
        results.qps = 1.0 / np.mean(latencies)

        print(f"  Avg Latency: {results.avg_latency_ms:.2f}ms")
        print(f"  P50 Latency: {results.p50_latency_ms:.2f}ms")
        print(f"  P95 Latency: {results.p95_latency_ms:.2f}ms")
        print(f"  P99 Latency: {results.p99_latency_ms:.2f}ms")
        print(f"  Throughput: {results.qps:.1f} QPS")

        conn.close()
        return results

    except Exception as e:
        print(f"ERROR: {e}")
        print("Make sure pgvectorscale is running: docker-compose up -d")
        return None


def bench_velesdb(config: BenchConfig, data: np.ndarray, queries: np.ndarray) -> Optional[BenchResults]:
    """Benchmark VelesDB with native Python SDK."""
    try:
        import velesdb
    except ImportError:
        print("ERROR: velesdb not installed. Run: pip install velesdb")
        return None

    print("\n" + "=" * 60)
    print("VELESDB (Native Python SDK - HNSW)")
    print("=" * 60)

    results = BenchResults()

    try:
        # Cleanup
        if os.path.exists(config.veles_data_dir):
            shutil.rmtree(config.veles_data_dir)

        # Setup
        print("Opening VelesDB database...")
        db = velesdb.Database(config.veles_data_dir)
        collection = db.create_collection("bench_vectors", dimension=config.dim, metric=config.metric)

        # Insertion with upsert_bulk (optimized)
        print(f"Inserting {len(data)} vectors with upsert_bulk()...")
        points = [{"id": i, "vector": v.tolist()} for i, v in enumerate(data)]

        start = time.time()
        # Check if upsert_bulk exists, fallback to upsert
        if hasattr(collection, 'upsert_bulk'):
            collection.upsert_bulk(points)
        else:
            # Fallback for older versions
            for i in range(0, len(points), config.batch_size):
                collection.upsert(points[i:i + config.batch_size])
        results.insert_time = time.time() - start
        print(f"  Insert time: {results.insert_time:.3f}s ({len(data) / results.insert_time:.0f} vec/s)")

        # Index time (included in upsert_bulk for HNSW)
        results.index_time = 0.0
        print("  Index build: included in insertion (parallel HNSW)")

        # Search
        print(f"Running {len(queries)} search queries...")
        # Warmup
        for q in queries[:10]:
            collection.search(q.tolist(), top_k=config.top_k)

        latencies = []
        for q in queries:
            start = time.time()
            collection.search(q.tolist(), top_k=config.top_k)
            latencies.append(time.time() - start)

        results.avg_latency_ms = np.mean(latencies) * 1000
        results.p50_latency_ms = np.percentile(latencies, 50) * 1000
        results.p95_latency_ms = np.percentile(latencies, 95) * 1000
        results.p99_latency_ms = np.percentile(latencies, 99) * 1000
        results.qps = 1.0 / np.mean(latencies)

        print(f"  Avg Latency: {results.avg_latency_ms:.2f}ms")
        print(f"  P50 Latency: {results.p50_latency_ms:.2f}ms")
        print(f"  P95 Latency: {results.p95_latency_ms:.2f}ms")
        print(f"  P99 Latency: {results.p99_latency_ms:.2f}ms")
        print(f"  Throughput: {results.qps:.1f} QPS")

        return results

    except Exception as e:
        print(f"ERROR: {e}")
        import traceback
        traceback.print_exc()
        return None


def print_comparison(config: BenchConfig, pg: BenchResults, veles: BenchResults):
    """Print comparison table."""
    print("\n" + "=" * 80)
    print("FINAL COMPARISON - VelesDB vs pgvectorscale")
    print("=" * 80)
    print(f"Dataset: {config.n_vectors} vectors, {config.dim} dimensions, {config.n_queries} queries")
    print("-" * 80)

    headers = ["Metric", "PGVectorScale", "VelesDB", "Speedup"]
    rows = []

    # Total Ingest
    pg_total = pg.insert_time + pg.index_time
    vl_total = veles.insert_time + veles.index_time
    speedup = pg_total / vl_total if vl_total > 0 else 0
    winner = "VelesDB" if vl_total < pg_total else "PGVectorScale"
    rows.append(["Total Ingest (s)", f"{pg_total:.3f}", f"{vl_total:.3f}", f"{speedup:.1f}x ({winner})"])

    # Avg Latency
    speedup = pg.avg_latency_ms / veles.avg_latency_ms if veles.avg_latency_ms > 0 else 0
    winner = "VelesDB" if veles.avg_latency_ms < pg.avg_latency_ms else "PGVectorScale"
    rows.append(["Avg Latency (ms)", f"{pg.avg_latency_ms:.2f}", f"{veles.avg_latency_ms:.2f}", f"{speedup:.1f}x ({winner})"])

    # P95 Latency
    speedup = pg.p95_latency_ms / veles.p95_latency_ms if veles.p95_latency_ms > 0 else 0
    winner = "VelesDB" if veles.p95_latency_ms < pg.p95_latency_ms else "PGVectorScale"
    rows.append(["P95 Latency (ms)", f"{pg.p95_latency_ms:.2f}", f"{veles.p95_latency_ms:.2f}", f"{speedup:.1f}x ({winner})"])

    # P99 Latency
    speedup = pg.p99_latency_ms / veles.p99_latency_ms if veles.p99_latency_ms > 0 else 0
    winner = "VelesDB" if veles.p99_latency_ms < pg.p99_latency_ms else "PGVectorScale"
    rows.append(["P99 Latency (ms)", f"{pg.p99_latency_ms:.2f}", f"{veles.p99_latency_ms:.2f}", f"{speedup:.1f}x ({winner})"])

    # QPS
    speedup = veles.qps / pg.qps if pg.qps > 0 else 0
    winner = "VelesDB" if veles.qps > pg.qps else "PGVectorScale"
    rows.append(["Throughput (QPS)", f"{pg.qps:.1f}", f"{veles.qps:.1f}", f"{speedup:.1f}x ({winner})"])

    # Print table
    col_widths = [22, 16, 16, 25]
    header_line = " | ".join(h.ljust(w) for h, w in zip(headers, col_widths))
    print(header_line)
    print("-" * len(header_line))
    for row in rows:
        print(" | ".join(str(c).ljust(w) for c, w in zip(row, col_widths)))

    print("=" * 80)


def main():
    parser = argparse.ArgumentParser(description="VelesDB vs pgvectorscale Benchmark")
    parser.add_argument("--vectors", type=int, default=10000, help="Number of vectors")
    parser.add_argument("--dim", type=int, default=768, help="Vector dimension")
    parser.add_argument("--queries", type=int, default=100, help="Number of queries")
    parser.add_argument("--skip-pg", action="store_true", help="Skip pgvectorscale benchmark")
    parser.add_argument("--skip-veles", action="store_true", help="Skip VelesDB benchmark")
    args = parser.parse_args()

    config = BenchConfig(
        n_vectors=args.vectors,
        dim=args.dim,
        n_queries=args.queries
    )

    # Generate data
    data = generate_normalized_vectors(config.n_vectors, config.dim)
    queries = generate_normalized_vectors(config.n_queries, config.dim)

    results = {}

    # Wait for services
    print("Waiting for services to be ready...")
    time.sleep(2)

    # Benchmark pgvectorscale
    if not args.skip_pg:
        results["pg"] = bench_pgvectorscale(config, data, queries)
        gc.collect()
        time.sleep(1)

    # Benchmark VelesDB
    if not args.skip_veles:
        results["veles"] = bench_velesdb(config, data, queries)

    # Print comparison
    if results.get("pg") and results.get("veles"):
        print_comparison(config, results["pg"], results["veles"])
    elif results.get("veles"):
        print("\n[VelesDB-only results - pgvectorscale skipped or failed]")
    elif results.get("pg"):
        print("\n[pgvectorscale-only results - VelesDB skipped or failed]")
    else:
        print("\nNo benchmark results available.")
        sys.exit(1)


if __name__ == "__main__":
    main()
