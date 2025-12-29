#!/usr/bin/env python3
"""
VelesDB vs pgvector - Recall Benchmark
======================================

IMPORTANT: Architecture Comparison
----------------------------------
- VelesDB: Native Python SDK (PyO3) - NO network overhead
- pgvector: Docker + PostgreSQL + psycopg2 - ~50ms network/SQL overhead

This benchmark compares:
1. RECALL accuracy (both use HNSW algorithm)
2. END-TO-END latency (including architecture overhead)

For a pure HNSW algorithm comparison, the recall numbers are relevant.
For real-world deployment comparison, the latency numbers are relevant.

VelesDB's advantage is ARCHITECTURAL: no SQL parsing, no network stack.
"""

import argparse
import time
import os
import shutil
import numpy as np
from typing import List

def generate_clustered_data(n_vectors: int, dim: int, n_clusters: int = 50) -> np.ndarray:
    """Génère des données avec clusters (plus réaliste que aléatoire uniforme)."""
    print(f"Generating {n_vectors} vectors with {n_clusters} clusters (dim={dim})...")
    
    vectors_per_cluster = n_vectors // n_clusters
    data = []
    
    np.random.seed(42)
    for c in range(n_clusters):
        # Centre du cluster
        center = np.random.randn(dim).astype('float32')
        center = center / np.linalg.norm(center)
        
        # Vecteurs autour du centre avec du bruit
        for _ in range(vectors_per_cluster):
            noise = np.random.randn(dim).astype('float32') * 0.1
            vec = center + noise
            vec = vec / np.linalg.norm(vec)  # Normaliser
            data.append(vec)
    
    # Compléter si nécessaire
    while len(data) < n_vectors:
        vec = np.random.randn(dim).astype('float32')
        data.append(vec / np.linalg.norm(vec))
    
    return np.array(data[:n_vectors], dtype='float32')


def brute_force_search(data: np.ndarray, query: np.ndarray, k: int) -> List[int]:
    """Recherche exacte par brute-force (ground truth)."""
    similarities = np.dot(data, query)
    return np.argsort(similarities)[-k:][::-1].tolist()


def compute_recall(ground_truth: List[int], predictions: List[int]) -> float:
    """Calcule Recall@k = |GT ∩ Pred| / k"""
    return len(set(ground_truth) & set(predictions)) / len(ground_truth)


def test_velesdb(data: np.ndarray, queries: np.ndarray, ground_truth: List[List[int]], 
                 dim: int, top_k: int) -> dict:
    """Test VelesDB avec affichage détaillé."""
    import velesdb
    
    print("\n" + "=" * 60)
    print("VELESDB RECALL TEST")
    print("=" * 60)
    
    data_dir = "./recall_bench_v2"
    if os.path.exists(data_dir):
        shutil.rmtree(data_dir)
    
    db = velesdb.Database(data_dir)
    coll = db.create_collection("vectors", dimension=dim, metric="cosine")
    
    # Insert
    print(f"Inserting {len(data)} vectors...")
    start = time.time()
    points = [{"id": i, "vector": v.tolist()} for i, v in enumerate(data)]
    coll.upsert_bulk(points)
    insert_time = time.time() - start
    print(f"  Insert time: {insert_time:.2f}s")
    
    # Search
    print(f"Running {len(queries)} queries...")
    recalls = []
    latencies = []
    
    for i, q in enumerate(queries):
        start = time.time()
        results = coll.search(q.tolist(), top_k=top_k)
        latencies.append(time.time() - start)
        
        pred_ids = [r['id'] for r in results]
        recall = compute_recall(ground_truth[i], pred_ids)
        recalls.append(recall)
    
    avg_recall = np.mean(recalls) * 100
    p50_latency = np.percentile(latencies, 50) * 1000
    p99_latency = np.percentile(latencies, 99) * 1000
    
    print(f"\n📊 VelesDB Results:")
    print(f"  Recall@{top_k}: {avg_recall:.1f}%")
    print(f"  Latency P50: {p50_latency:.2f}ms")
    print(f"  Latency P99: {p99_latency:.2f}ms")
    
    # Cleanup (ignore errors on Windows due to file locks)
    del coll
    del db
    try:
        shutil.rmtree(data_dir)
    except PermissionError:
        pass  # Windows file lock, will be cleaned up later
    
    return {
        "recall": avg_recall,
        "latency_p50_ms": p50_latency,
        "latency_p99_ms": p99_latency,
        "insert_time_s": insert_time
    }


def test_pgvector(data: np.ndarray, queries: np.ndarray, ground_truth: List[List[int]],
                  dim: int, top_k: int, ef_search: int = 100) -> dict:
    """Test pgvector HNSW."""
    try:
        import psycopg2
    except ImportError:
        print("psycopg2 not installed, skipping pgvector test")
        return None
    
    print("\n" + "=" * 60)
    print("PGVECTOR (HNSW) RECALL TEST")
    print("=" * 60)
    
    pg_url = os.environ.get("PG_URL", "postgres://postgres:password@localhost:5433/postgres")
    
    try:
        conn = psycopg2.connect(pg_url)
        cur = conn.cursor()
        
        # Setup
        cur.execute("CREATE EXTENSION IF NOT EXISTS vector;")
        cur.execute("DROP TABLE IF EXISTS bench_vectors;")
        cur.execute(f"CREATE TABLE bench_vectors (id serial PRIMARY KEY, embedding vector({dim}));")
        conn.commit()
        
        # Insert
        print(f"Inserting {len(data)} vectors...")
        start = time.time()
        batch_size = 1000
        for i in range(0, len(data), batch_size):
            batch = data[i:i + batch_size]
            args_str = ','.join(
                cur.mogrify("(%s::vector)", (v.tolist(),)).decode('utf-8')
                for v in batch
            )
            cur.execute("INSERT INTO bench_vectors (embedding) VALUES " + args_str)
        conn.commit()
        insert_time = time.time() - start
        print(f"  Insert time: {insert_time:.2f}s")
        
        # Create index
        print(f"Building HNSW index (m=16, ef_construction=200)...")
        cur.execute("""
            CREATE INDEX ON bench_vectors 
            USING hnsw (embedding vector_cosine_ops)
            WITH (m = 16, ef_construction = 200);
        """)
        conn.commit()
        
        # Set ef_search
        cur.execute(f"SET hnsw.ef_search = {ef_search};")
        
        # Search
        print(f"Running {len(queries)} queries (ef_search={ef_search})...")
        recalls = []
        latencies = []
        
        for i, q in enumerate(queries):
            start = time.time()
            cur.execute(
                "SELECT id FROM bench_vectors ORDER BY embedding <=> %s::vector LIMIT %s",
                (q.tolist(), top_k)
            )
            results = cur.fetchall()
            latencies.append(time.time() - start)
            
            pred_ids = [r[0] - 1 for r in results]  # PostgreSQL 1-indexed
            recall = compute_recall(ground_truth[i], pred_ids)
            recalls.append(recall)
        
        conn.close()
        
        avg_recall = np.mean(recalls) * 100
        p50_latency = np.percentile(latencies, 50) * 1000
        p99_latency = np.percentile(latencies, 99) * 1000
        
        print(f"\n📊 pgvector Results:")
        print(f"  Recall@{top_k}: {avg_recall:.1f}%")
        print(f"  Latency P50: {p50_latency:.2f}ms")
        print(f"  Latency P99: {p99_latency:.2f}ms")
        
        return {
            "recall": avg_recall,
            "latency_p50_ms": p50_latency,
            "latency_p99_ms": p99_latency,
            "insert_time_s": insert_time
        }
        
    except Exception as e:
        print(f"ERROR: {e}")
        return None


def main():
    parser = argparse.ArgumentParser(description="VelesDB Recall Benchmark v2")
    parser.add_argument("--vectors", type=int, default=10000)
    parser.add_argument("--dim", type=int, default=768)
    parser.add_argument("--queries", type=int, default=100)
    parser.add_argument("--top-k", type=int, default=10)
    parser.add_argument("--clusters", type=int, default=50, help="Number of data clusters")
    parser.add_argument("--velesdb-only", action="store_true")
    args = parser.parse_args()
    
    # Generate data with clusters (more realistic)
    data = generate_clustered_data(args.vectors, args.dim, args.clusters)
    
    # Queries from data (ensures meaningful results)
    np.random.seed(123)
    query_indices = np.random.choice(len(data), args.queries, replace=False)
    queries = data[query_indices]
    
    # Ground truth via brute-force
    print(f"\nComputing ground truth (brute-force top-{args.top_k})...")
    ground_truth = [brute_force_search(data, q, args.top_k) for q in queries]
    print("Done.")
    
    # Test VelesDB
    vdb = test_velesdb(data, queries, ground_truth, args.dim, args.top_k)
    
    # Test pgvector
    pg = None
    if not args.velesdb_only:
        pg = test_pgvector(data, queries, ground_truth, args.dim, args.top_k, ef_search=100)
    
    # Summary
    print("\n" + "=" * 70)
    print("SUMMARY")
    print("=" * 70)
    print(f"Dataset: {args.vectors} vectors, {args.dim}D, {args.clusters} clusters")
    print(f"Queries: {args.queries}, Top-K: {args.top_k}")
    print("-" * 70)
    
    print("\n📊 RECALL (HNSW Algorithm Quality):")
    if vdb:
        print(f"  VelesDB:  {vdb['recall']:.1f}%")
    if pg:
        print(f"  pgvector: {pg['recall']:.1f}%")
    
    print("\n⏱️ LATENCY (End-to-End, includes architecture overhead):")
    if vdb:
        print(f"  VelesDB (native):     P50={vdb['latency_p50_ms']:.1f}ms  P99={vdb['latency_p99_ms']:.1f}ms")
    if pg:
        print(f"  pgvector (Docker+SQL): P50={pg['latency_p50_ms']:.1f}ms  P99={pg['latency_p99_ms']:.1f}ms")
    
    print("\n⚠️ NOTE: Latency difference is primarily ARCHITECTURAL:")
    print("   - VelesDB: Native Python calls (PyO3), no network")
    print("   - pgvector: Docker + PostgreSQL + SQL parsing + network")
    
    if vdb and pg:
        speedup = pg['latency_p50_ms'] / vdb['latency_p50_ms']
        recall_diff = vdb['recall'] - pg['recall']
        print(f"\n📈 VelesDB: {speedup:.0f}x faster latency, {recall_diff:+.1f}% recall difference")


if __name__ == "__main__":
    main()
