#!/usr/bin/env python3
"""
VelesDB vs pgvector - All Quality Modes Benchmark
==================================================

Tests all VelesDB search quality modes against pgvector.
"""

import time
import argparse
import numpy as np
import requests
from typing import List, Optional, Dict
import psycopg2

# VelesDB Quality Modes (ef_search values matching SearchQuality enum)
QUALITY_MODES = {
    "Fast": 64,
    "Balanced": 128,
    "Accurate": 256,
    "HighRecall": 1024,   # Increased from 512 for ≥95% recall
    "Perfect": 2048,      # Brute-force SIMD for 100% recall
}


def generate_clustered_data(n_vectors: int, dim: int, n_clusters: int = 50, seed: int = 42):
    """Generate clustered data for realistic benchmark."""
    np.random.seed(seed)
    centers = np.random.randn(n_clusters, dim).astype(np.float32)
    centers = centers / np.linalg.norm(centers, axis=1, keepdims=True)
    
    data = []
    labels = []
    vectors_per_cluster = n_vectors // n_clusters
    
    for i in range(n_clusters):
        noise = np.random.randn(vectors_per_cluster, dim).astype(np.float32) * 0.1
        cluster_vectors = centers[i] + noise
        cluster_vectors = cluster_vectors / np.linalg.norm(cluster_vectors, axis=1, keepdims=True)
        data.append(cluster_vectors)
        labels.extend([i] * vectors_per_cluster)
    
    return np.vstack(data), np.array(labels)


def compute_ground_truth(data: np.ndarray, queries: np.ndarray, k: int = 10):
    """Compute ground truth using brute-force."""
    ground_truth = []
    for q in queries:
        similarities = data @ q
        top_k_indices = np.argsort(similarities)[-k:][::-1]
        ground_truth.append(top_k_indices.tolist())
    return ground_truth


def compute_recall(truth: List[int], predicted: List[int]) -> float:
    """Compute recall@k."""
    if not truth or not predicted:
        return 0.0
    return len(set(truth) & set(predicted)) / len(truth)


def test_velesdb_mode(data: np.ndarray, queries: np.ndarray, ground_truth: List[List[int]],
                      dim: int, top_k: int, ef_search: int, mode_name: str) -> Optional[Dict]:
    """Test VelesDB with specific ef_search value."""
    base_url = "http://localhost:8080"
    collection_name = f"bench_mode_{mode_name.lower()}"
    
    try:
        session = requests.Session()
        
        # Check health
        resp = session.get(f"{base_url}/health", timeout=5)
        if resp.status_code != 200:
            print(f"  [{mode_name}] VelesDB not healthy")
            return None
        
        # Delete if exists
        session.delete(f"{base_url}/collections/{collection_name}")
        
        # Create collection
        resp = session.post(f"{base_url}/collections", json={
            "name": collection_name,
            "dimension": dim,
            "metric": "cosine"
        })
        
        # Insert vectors in batches
        start = time.time()
        batch_size = 1000
        for batch_start in range(0, len(data), batch_size):
            batch_end = min(batch_start + batch_size, len(data))
            points = [{"id": i, "vector": data[i].tolist()} for i in range(batch_start, batch_end)]
            resp = session.post(f"{base_url}/collections/{collection_name}/points", json={"points": points})
            if resp.status_code not in [200, 201]:
                print(f"    Insert error: {resp.text}")
        insert_time = time.time() - start
        
        # Warmup
        for _ in range(3):
            session.post(f"{base_url}/collections/{collection_name}/search", json={
                "vector": queries[0].tolist(),
                "top_k": top_k,
                "ef_search": ef_search
            })
        
        # Search
        recalls = []
        latencies = []
        
        for i, q in enumerate(queries):
            start = time.time()
            resp = session.post(f"{base_url}/collections/{collection_name}/search", json={
                "vector": q.tolist(),
                "top_k": top_k,
                "ef_search": ef_search
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
        
        return {
            "mode": mode_name,
            "ef_search": ef_search,
            "recall": np.mean(recalls) * 100,
            "latency_p50_ms": np.percentile(latencies, 50) * 1000,
            "latency_p99_ms": np.percentile(latencies, 99) * 1000,
            "insert_time_s": insert_time
        }
        
    except Exception as e:
        print(f"  [{mode_name}] ERROR: {e}")
        return None


def test_pgvector(data: np.ndarray, queries: np.ndarray, ground_truth: List[List[int]],
                  dim: int, top_k: int) -> Optional[Dict]:
    """Test pgvector."""
    try:
        conn = psycopg2.connect(
            host="localhost",
            port=5433,
            database="benchmark",
            user="benchmark",
            password="benchmark"
        )
        conn.autocommit = True
        cur = conn.cursor()
        
        # Setup
        cur.execute("DROP TABLE IF EXISTS vectors")
        cur.execute("CREATE EXTENSION IF NOT EXISTS vector")
        cur.execute(f"""
            CREATE TABLE vectors (
                id SERIAL PRIMARY KEY,
                embedding vector({dim})
            )
        """)
        
        # Insert
        start = time.time()
        for i, vec in enumerate(data):
            cur.execute(
                "INSERT INTO vectors (id, embedding) VALUES (%s, %s)",
                (i, vec.tolist())
            )
        insert_time = time.time() - start
        
        # Build HNSW index
        start = time.time()
        cur.execute("""
            CREATE INDEX ON vectors USING hnsw (embedding vector_cosine_ops)
            WITH (m = 16, ef_construction = 200)
        """)
        index_time = time.time() - start
        
        # Warmup
        for _ in range(3):
            cur.execute("SET hnsw.ef_search = 100")
            cur.execute("""
                SELECT id, 1 - (embedding <=> %s::vector) as similarity
                FROM vectors ORDER BY embedding <=> %s::vector LIMIT %s
            """, (queries[0].tolist(), queries[0].tolist(), top_k))
        
        # Search
        recalls = []
        latencies = []
        
        for i, q in enumerate(queries):
            start = time.time()
            cur.execute("""
                SELECT id FROM vectors
                ORDER BY embedding <=> %s::vector LIMIT %s
            """, (q.tolist(), top_k))
            latencies.append(time.time() - start)
            
            pred_ids = [row[0] for row in cur.fetchall()]
            recall = compute_recall(ground_truth[i], pred_ids)
            recalls.append(recall)
        
        cur.close()
        conn.close()
        
        return {
            "mode": "pgvector",
            "recall": np.mean(recalls) * 100,
            "latency_p50_ms": np.percentile(latencies, 50) * 1000,
            "latency_p99_ms": np.percentile(latencies, 99) * 1000,
            "insert_time_s": insert_time,
            "index_time_s": index_time
        }
        
    except Exception as e:
        print(f"  [pgvector] ERROR: {e}")
        return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--vectors", type=int, default=100000)
    parser.add_argument("--dim", type=int, default=768)
    parser.add_argument("--queries", type=int, default=100)
    args = parser.parse_args()
    
    print("=" * 70)
    print("VELESDB ALL QUALITY MODES vs PGVECTOR")
    print("=" * 70)
    print(f"Dataset: {args.vectors} vectors, {args.dim}D\n")
    
    # Generate data
    print("Generating data...")
    data, _ = generate_clustered_data(args.vectors, args.dim)
    queries = np.random.randn(args.queries, args.dim).astype(np.float32)
    queries = queries / np.linalg.norm(queries, axis=1, keepdims=True)
    
    print("Computing ground truth...")
    ground_truth = compute_ground_truth(data, queries)
    print("Done.\n")
    
    results = []
    
    # Test all VelesDB modes
    print("Testing VelesDB modes...")
    for mode_name, ef_search in QUALITY_MODES.items():
        print(f"  Testing {mode_name} (ef_search={ef_search})...")
        result = test_velesdb_mode(data, queries, ground_truth, args.dim, 10, ef_search, mode_name)
        if result:
            results.append(result)
            print(f"    Recall: {result['recall']:.1f}%, Latency P50: {result['latency_p50_ms']:.1f}ms")
    
    # Test pgvector
    print("\nTesting pgvector...")
    pg_result = test_pgvector(data, queries, ground_truth, args.dim, 10)
    if pg_result:
        results.append(pg_result)
        print(f"    Recall: {pg_result['recall']:.1f}%, Latency P50: {pg_result['latency_p50_ms']:.1f}ms")
    
    # Summary table
    print("\n" + "=" * 70)
    print("RESULTS SUMMARY")
    print("=" * 70)
    print(f"{'Mode':<12} {'ef_search':<10} {'Recall@10':<12} {'P50 (ms)':<12} {'P99 (ms)':<12}")
    print("-" * 70)
    
    for r in results:
        ef = r.get('ef_search', 'N/A')
        print(f"{r['mode']:<12} {str(ef):<10} {r['recall']:>8.1f}%    {r['latency_p50_ms']:>8.1f}     {r['latency_p99_ms']:>8.1f}")
    
    print("-" * 70)
    
    # Find best VelesDB mode matching pgvector recall
    if pg_result:
        pg_recall = pg_result['recall']
        print(f"\npgvector recall: {pg_recall:.1f}%")
        for r in results:
            if r['mode'] != 'pgvector' and r['recall'] >= pg_recall - 1:
                print(f"Best VelesDB match: {r['mode']} ({r['recall']:.1f}% recall)")
                break


if __name__ == "__main__":
    main()
