#!/usr/bin/env python3
"""
VelesDB vs pgvector - Fair Docker Benchmark
============================================

FAIR COMPARISON: Both databases run in Docker with network overhead.

- VelesDB: REST API via HTTP (localhost:8080)
- pgvector: PostgreSQL via psycopg2 (localhost:5433)

This measures real-world performance when both are deployed as services.
"""

import argparse
import time
import os
import requests
import numpy as np
from typing import List, Optional

def generate_clustered_data(n_vectors: int, dim: int, n_clusters: int = 50) -> np.ndarray:
    """Generate clustered vectors (realistic embeddings)."""
    print(f"Generating {n_vectors} vectors with {n_clusters} clusters (dim={dim})...")
    
    vectors_per_cluster = n_vectors // n_clusters
    data = []
    
    np.random.seed(42)
    for c in range(n_clusters):
        center = np.random.randn(dim).astype('float32')
        center = center / np.linalg.norm(center)
        
        for _ in range(vectors_per_cluster):
            noise = np.random.randn(dim).astype('float32') * 0.1
            vec = center + noise
            vec = vec / np.linalg.norm(vec)
            data.append(vec)
    
    while len(data) < n_vectors:
        vec = np.random.randn(dim).astype('float32')
        data.append(vec / np.linalg.norm(vec))
    
    return np.array(data[:n_vectors], dtype='float32')


def brute_force_search(data: np.ndarray, query: np.ndarray, k: int) -> List[int]:
    """Exact brute-force search (ground truth)."""
    similarities = np.dot(data, query)
    return np.argsort(similarities)[-k:][::-1].tolist()


def compute_recall(ground_truth: List[int], predictions: List[int]) -> float:
    """Compute Recall@k."""
    return len(set(ground_truth) & set(predictions)) / len(ground_truth)


def test_velesdb_rest(data: np.ndarray, queries: np.ndarray, ground_truth: List[List[int]],
                      dim: int, top_k: int, base_url: str = "http://localhost:8080") -> Optional[dict]:
    """Test VelesDB via REST API (Docker)."""
    print("\n" + "=" * 60)
    print("VELESDB REST API TEST (Docker)")
    print("=" * 60)
    
    session = requests.Session()
    collection_name = "bench_vectors"
    
    try:
        # Health check
        resp = session.get(f"{base_url}/health", timeout=5)
        if resp.status_code != 200:
            print(f"ERROR: VelesDB not healthy: {resp.status_code}")
            return None
        print("VelesDB server is healthy")
        
        # Delete collection if exists
        session.delete(f"{base_url}/collections/{collection_name}")
        
        # Create collection
        resp = session.post(f"{base_url}/collections", json={
            "name": collection_name,
            "dimension": dim,
            "metric": "cosine"
        })
        if resp.status_code not in [200, 201]:
            print(f"ERROR creating collection: {resp.text}")
            return None
        
        # Insert vectors in batches (larger batches for fair comparison with pgvector)
        print(f"Inserting {len(data)} vectors...")
        start = time.time()
        batch_size = 5000  # Single batch = minimal HTTP overhead
        for i in range(0, len(data), batch_size):
            batch = data[i:i + batch_size]
            points = [{"id": i + j, "vector": v.tolist()} for j, v in enumerate(batch)]
            resp = session.post(f"{base_url}/collections/{collection_name}/points", json={"points": points})
            if resp.status_code not in [200, 201]:
                print(f"ERROR inserting: {resp.text}")
                return None
        insert_time = time.time() - start
        print(f"  Insert time: {insert_time:.2f}s")
        
        # Warmup
        for _ in range(5):
            session.post(f"{base_url}/collections/{collection_name}/search", json={
                "vector": queries[0].tolist(),
                "top_k": top_k,
                "ef_search": 512
            })
        
        # Search and measure
        print(f"Running {len(queries)} queries...")
        recalls = []
        latencies = []
        
        for i, q in enumerate(queries):
            start = time.time()
            resp = session.post(f"{base_url}/collections/{collection_name}/search", json={
                "vector": q.tolist(),
                "top_k": top_k,
                "ef_search": 512  # HighRecall mode for fair comparison
            })
            latencies.append(time.time() - start)
            
            if resp.status_code == 200:
                results = resp.json()
                pred_ids = [r["id"] for r in results.get("results", results)]
                recall = compute_recall(ground_truth[i], pred_ids)
                recalls.append(recall)
            else:
                recalls.append(0)
        
        # Cleanup
        session.delete(f"{base_url}/collections/{collection_name}")
        
        avg_recall = np.mean(recalls) * 100
        p50_latency = np.percentile(latencies, 50) * 1000
        p99_latency = np.percentile(latencies, 99) * 1000
        
        print(f"\n📊 VelesDB REST Results:")
        print(f"  Recall@{top_k}: {avg_recall:.1f}%")
        print(f"  Latency P50: {p50_latency:.2f}ms")
        print(f"  Latency P99: {p99_latency:.2f}ms")
        
        return {
            "recall": avg_recall,
            "latency_p50_ms": p50_latency,
            "latency_p99_ms": p99_latency,
            "insert_time_s": insert_time
        }
        
    except requests.exceptions.ConnectionError:
        print("ERROR: Cannot connect to VelesDB. Run: docker-compose up -d")
        return None
    except Exception as e:
        print(f"ERROR: {e}")
        return None


def test_pgvector(data: np.ndarray, queries: np.ndarray, ground_truth: List[List[int]],
                  dim: int, top_k: int, ef_search: int = 100) -> Optional[dict]:
    """Test pgvector via Docker."""
    try:
        import psycopg2
    except ImportError:
        print("ERROR: psycopg2 not installed")
        return None
    
    print("\n" + "=" * 60)
    print("PGVECTOR TEST (Docker)")
    print("=" * 60)
    
    pg_url = os.environ.get("PG_URL", "postgres://postgres:benchpass@localhost:5433/benchmark")
    
    try:
        conn = psycopg2.connect(pg_url)
        cur = conn.cursor()
        
        # Setup
        print("Setting up PostgreSQL...")
        cur.execute("CREATE EXTENSION IF NOT EXISTS vector;")
        cur.execute("DROP TABLE IF EXISTS bench_vectors;")
        cur.execute(f"CREATE TABLE bench_vectors (id serial PRIMARY KEY, embedding vector({dim}));")
        conn.commit()
        
        # Insert + Index (measure total time for fair comparison with VelesDB)
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
        raw_insert_time = time.time() - start
        print(f"  Raw insert time: {raw_insert_time:.2f}s")
        
        # Create HNSW index (this is part of the "insert" cost - VelesDB does it inline)
        print("Building HNSW index...")
        index_start = time.time()
        cur.execute("""
            CREATE INDEX ON bench_vectors 
            USING hnsw (embedding vector_cosine_ops)
            WITH (m = 16, ef_construction = 200);
        """)
        conn.commit()
        index_time = time.time() - index_start
        insert_time = time.time() - start  # Total: insert + index
        print(f"  Index build time: {index_time:.2f}s")
        print(f"  Total insert+index time: {insert_time:.2f}s")
        
        cur.execute(f"SET hnsw.ef_search = {ef_search};")
        
        # Warmup
        for _ in range(5):
            cur.execute(
                "SELECT id FROM bench_vectors ORDER BY embedding <=> %s::vector LIMIT %s",
                (queries[0].tolist(), top_k)
            )
            cur.fetchall()
        
        # Search
        print(f"Running {len(queries)} queries...")
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
            
            pred_ids = [r[0] - 1 for r in results]
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
        print("Make sure pgvector is running: docker-compose up -d")
        return None


def main():
    parser = argparse.ArgumentParser(description="VelesDB vs pgvector - Fair Docker Benchmark")
    parser.add_argument("--vectors", type=int, default=10000)
    parser.add_argument("--dim", type=int, default=768)
    parser.add_argument("--queries", type=int, default=100)
    parser.add_argument("--top-k", type=int, default=10)
    parser.add_argument("--clusters", type=int, default=50)
    parser.add_argument("--velesdb-url", default="http://localhost:8080")
    args = parser.parse_args()
    
    print("=" * 70)
    print("FAIR COMPARISON: Both databases running in Docker")
    print("=" * 70)
    
    # Generate data
    data = generate_clustered_data(args.vectors, args.dim, args.clusters)
    
    # Queries from data
    np.random.seed(123)
    query_indices = np.random.choice(len(data), args.queries, replace=False)
    queries = data[query_indices]
    
    # Ground truth
    print(f"\nComputing ground truth (brute-force top-{args.top_k})...")
    ground_truth = [brute_force_search(data, q, args.top_k) for q in queries]
    print("Done.")
    
    # Test both
    vdb = test_velesdb_rest(data, queries, ground_truth, args.dim, args.top_k, args.velesdb_url)
    pg = test_pgvector(data, queries, ground_truth, args.dim, args.top_k)
    
    # Summary
    print("\n" + "=" * 70)
    print("FAIR COMPARISON SUMMARY (Both via Docker/Network)")
    print("=" * 70)
    print(f"Dataset: {args.vectors} vectors, {args.dim}D, {args.clusters} clusters")
    print("-" * 70)
    
    print("\n📊 RECALL (HNSW Algorithm Quality):")
    if vdb:
        print(f"  VelesDB REST: {vdb['recall']:.1f}%")
    if pg:
        print(f"  pgvector:     {pg['recall']:.1f}%")
    
    print("\n⏱️ LATENCY (Both via network):")
    if vdb:
        print(f"  VelesDB REST: P50={vdb['latency_p50_ms']:.1f}ms  P99={vdb['latency_p99_ms']:.1f}ms")
    if pg:
        print(f"  pgvector:     P50={pg['latency_p50_ms']:.1f}ms  P99={pg['latency_p99_ms']:.1f}ms")
    
    if vdb and pg:
        speedup = pg['latency_p50_ms'] / vdb['latency_p50_ms'] if vdb['latency_p50_ms'] > 0 else 0
        recall_diff = vdb['recall'] - pg['recall']
        print(f"\n📈 VelesDB: {speedup:.1f}x faster, {recall_diff:+.1f}% recall difference")


if __name__ == "__main__":
    main()
