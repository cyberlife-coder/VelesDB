#!/usr/bin/env python3
"""
VelesDB 50M Vectors Benchmark Script
=====================================

Validates VelesDB performance projections at 50M vectors scale.

Requirements:
- 256GB+ RAM (or adjust VECTOR_COUNT)
- ~300GB disk space
- Python 3.10+
- Docker (for Qdrant comparison)

Usage:
    # Full benchmark (50M vectors)
    python benchmark_50m.py --full

    # Quick test (1M vectors)
    python benchmark_50m.py --quick

    # Custom scale
    python benchmark_50m.py --vectors 10000000

Author: Wiscale France (Julien Lange)
License: ELv2
"""

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import numpy as np

# Optional imports
try:
    import requests
    HAS_REQUESTS = True
except ImportError:
    HAS_REQUESTS = False

try:
    from qdrant_client import QdrantClient
    from qdrant_client.models import Distance, VectorParams, PointStruct
    HAS_QDRANT = True
except ImportError:
    HAS_QDRANT = False


# =============================================================================
# Configuration
# =============================================================================

@dataclass
class BenchmarkConfig:
    """Benchmark configuration."""
    vector_count: int = 50_000_000
    dimension: int = 768
    batch_size: int = 10_000
    search_queries: int = 1000
    top_k: int = 10
    warmup_queries: int = 100
    
    # Paths
    data_dir: Path = Path("./benchmark_data")
    velesdb_data: Path = Path("./velesdb_50m")
    
    # Servers
    velesdb_url: str = "http://localhost:8080"
    qdrant_url: str = "http://localhost:6333"


# =============================================================================
# Vector Generation
# =============================================================================

def generate_vectors(config: BenchmarkConfig) -> tuple[np.ndarray, np.ndarray]:
    """
    Generate random normalized vectors for benchmarking.
    
    Returns:
        Tuple of (dataset_vectors, query_vectors)
    """
    print(f"\n🔄 Generating {config.vector_count:,} vectors ({config.dimension}D)...")
    
    vectors_file = config.data_dir / f"vectors_{config.vector_count}.npy"
    queries_file = config.data_dir / f"queries_{config.search_queries}.npy"
    
    config.data_dir.mkdir(parents=True, exist_ok=True)
    
    # Check cache
    if vectors_file.exists() and queries_file.exists():
        print(f"   📂 Loading cached vectors from {vectors_file}")
        vectors = np.load(vectors_file)
        queries = np.load(queries_file)
        return vectors, queries
    
    # Estimate memory
    memory_gb = (config.vector_count * config.dimension * 4) / (1024**3)
    print(f"   ⚠️  This will use ~{memory_gb:.1f} GB of RAM")
    
    # Generate in batches to avoid memory issues
    print(f"   📊 Generating {config.vector_count:,} vectors in batches...")
    
    vectors = np.zeros((config.vector_count, config.dimension), dtype=np.float32)
    
    batch_count = config.vector_count // config.batch_size
    for i in range(batch_count):
        start_idx = i * config.batch_size
        end_idx = start_idx + config.batch_size
        
        # Generate random vectors
        batch = np.random.randn(config.batch_size, config.dimension).astype(np.float32)
        
        # L2 normalize
        norms = np.linalg.norm(batch, axis=1, keepdims=True)
        batch = batch / norms
        
        vectors[start_idx:end_idx] = batch
        
        if (i + 1) % 100 == 0:
            progress = (i + 1) / batch_count * 100
            print(f"   [{progress:5.1f}%] Generated {end_idx:,} vectors")
    
    # Generate query vectors
    print(f"   📊 Generating {config.search_queries} query vectors...")
    queries = np.random.randn(config.search_queries, config.dimension).astype(np.float32)
    queries = queries / np.linalg.norm(queries, axis=1, keepdims=True)
    
    # Save to disk
    print(f"   💾 Saving vectors to {vectors_file}")
    np.save(vectors_file, vectors)
    np.save(queries_file, queries)
    
    print(f"   ✅ Generated {config.vector_count:,} vectors")
    return vectors, queries


# =============================================================================
# VelesDB Benchmark
# =============================================================================

def start_velesdb_server(config: BenchmarkConfig) -> Optional[subprocess.Popen]:
    """Start VelesDB server if not running."""
    try:
        if HAS_REQUESTS:
            response = requests.get(f"{config.velesdb_url}/health", timeout=2)
            if response.status_code == 200:
                print("   ✅ VelesDB server already running")
                return None
    except Exception:
        pass
    
    print("   🚀 Starting VelesDB server...")
    config.velesdb_data.mkdir(parents=True, exist_ok=True)
    
    process = subprocess.Popen(
        ["velesdb-server", "--data-dir", str(config.velesdb_data), "--port", "8080"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE
    )
    
    # Wait for startup
    for _ in range(30):
        try:
            if HAS_REQUESTS:
                response = requests.get(f"{config.velesdb_url}/health", timeout=1)
                if response.status_code == 200:
                    print("   ✅ VelesDB server started")
                    return process
        except Exception:
            pass
        time.sleep(1)
    
    raise RuntimeError("VelesDB server failed to start")


def benchmark_velesdb(config: BenchmarkConfig, vectors: np.ndarray, queries: np.ndarray) -> dict:
    """
    Benchmark VelesDB search performance.
    
    Returns:
        Dict with latency percentiles and metadata
    """
    if not HAS_REQUESTS:
        print("   ⚠️  requests library not installed, skipping VelesDB benchmark")
        return {}
    
    print("\n🐺 Benchmarking VelesDB...")
    
    # Start server
    server_process = start_velesdb_server(config)
    
    try:
        # Create collection
        print(f"   📦 Creating collection with {config.vector_count:,} vectors...")
        
        collection_name = "benchmark_50m"
        
        # Delete if exists
        try:
            requests.delete(f"{config.velesdb_url}/collections/{collection_name}")
        except Exception:
            pass
        
        # Create collection
        response = requests.post(
            f"{config.velesdb_url}/collections",
            json={
                "name": collection_name,
                "dimension": config.dimension,
                "metric": "cosine"
            }
        )
        
        if response.status_code not in [200, 201]:
            print(f"   ❌ Failed to create collection: {response.text}")
            return {}
        
        # Insert vectors in batches
        print(f"   📥 Inserting {config.vector_count:,} vectors...")
        insert_start = time.time()
        
        for i in range(0, config.vector_count, config.batch_size):
            batch_vectors = vectors[i:i + config.batch_size]
            
            points = [
                {
                    "id": i + j,
                    "vector": batch_vectors[j].tolist(),
                    "payload": {"batch": i // config.batch_size}
                }
                for j in range(len(batch_vectors))
            ]
            
            response = requests.post(
                f"{config.velesdb_url}/collections/{collection_name}/points",
                json={"points": points}
            )
            
            if response.status_code not in [200, 201]:
                print(f"   ❌ Insert failed at batch {i}: {response.text}")
                return {}
            
            if (i // config.batch_size + 1) % 100 == 0:
                progress = (i + config.batch_size) / config.vector_count * 100
                print(f"   [{progress:5.1f}%] Inserted {i + config.batch_size:,} vectors")
        
        insert_time = time.time() - insert_start
        print(f"   ✅ Insert complete in {insert_time:.1f}s ({config.vector_count / insert_time:.0f} vec/s)")
        
        # Warmup
        print(f"   🔥 Warmup ({config.warmup_queries} queries)...")
        for i in range(config.warmup_queries):
            requests.post(
                f"{config.velesdb_url}/collections/{collection_name}/search",
                json={"vector": queries[i % len(queries)].tolist(), "top_k": config.top_k}
            )
        
        # Benchmark search
        print(f"   ⏱️  Running {config.search_queries} search queries...")
        latencies = []
        
        for i, query in enumerate(queries):
            start = time.perf_counter()
            
            response = requests.post(
                f"{config.velesdb_url}/collections/{collection_name}/search",
                json={"vector": query.tolist(), "top_k": config.top_k}
            )
            
            end = time.perf_counter()
            
            if response.status_code == 200:
                latencies.append((end - start) * 1000)  # Convert to ms
            
            if (i + 1) % 100 == 0:
                print(f"   [{(i+1)/config.search_queries*100:5.1f}%] Completed {i+1} queries")
        
        # Calculate percentiles
        latencies = np.array(latencies)
        results = {
            "database": "VelesDB",
            "vector_count": config.vector_count,
            "dimension": config.dimension,
            "queries": len(latencies),
            "latency_p50_ms": float(np.percentile(latencies, 50)),
            "latency_p95_ms": float(np.percentile(latencies, 95)),
            "latency_p99_ms": float(np.percentile(latencies, 99)),
            "latency_mean_ms": float(np.mean(latencies)),
            "latency_min_ms": float(np.min(latencies)),
            "latency_max_ms": float(np.max(latencies)),
            "insert_time_s": insert_time,
            "throughput_qps": len(latencies) / (sum(latencies) / 1000)
        }
        
        print(f"\n   📊 VelesDB Results:")
        print(f"      p50: {results['latency_p50_ms']:.2f}ms")
        print(f"      p95: {results['latency_p95_ms']:.2f}ms")
        print(f"      p99: {results['latency_p99_ms']:.2f}ms")
        print(f"      QPS: {results['throughput_qps']:.0f}")
        
        return results
    
    finally:
        if server_process:
            server_process.terminate()
            server_process.wait()


# =============================================================================
# Qdrant Benchmark (for comparison)
# =============================================================================

def benchmark_qdrant(config: BenchmarkConfig, vectors: np.ndarray, queries: np.ndarray) -> dict:
    """
    Benchmark Qdrant search performance for comparison.
    
    Returns:
        Dict with latency percentiles and metadata
    """
    if not HAS_QDRANT:
        print("\n⚠️  qdrant-client not installed, skipping Qdrant benchmark")
        print("   Install with: pip install qdrant-client")
        return {}
    
    print("\n🦀 Benchmarking Qdrant...")
    
    try:
        client = QdrantClient(url=config.qdrant_url)
        client.get_collections()
        print("   ✅ Qdrant server connected")
    except Exception as e:
        print(f"   ⚠️  Qdrant not available: {e}")
        print("   Start Qdrant with: docker run -p 6333:6333 qdrant/qdrant")
        return {}
    
    collection_name = "benchmark_50m"
    
    # Create collection
    print(f"   📦 Creating collection...")
    try:
        client.delete_collection(collection_name)
    except Exception:
        pass
    
    client.create_collection(
        collection_name=collection_name,
        vectors_config=VectorParams(
            size=config.dimension,
            distance=Distance.COSINE
        )
    )
    
    # Insert vectors
    print(f"   📥 Inserting {config.vector_count:,} vectors...")
    insert_start = time.time()
    
    for i in range(0, config.vector_count, config.batch_size):
        batch_vectors = vectors[i:i + config.batch_size]
        
        points = [
            PointStruct(
                id=i + j,
                vector=batch_vectors[j].tolist(),
                payload={"batch": i // config.batch_size}
            )
            for j in range(len(batch_vectors))
        ]
        
        client.upsert(collection_name=collection_name, points=points)
        
        if (i // config.batch_size + 1) % 100 == 0:
            progress = (i + config.batch_size) / config.vector_count * 100
            print(f"   [{progress:5.1f}%] Inserted {i + config.batch_size:,} vectors")
    
    insert_time = time.time() - insert_start
    print(f"   ✅ Insert complete in {insert_time:.1f}s")
    
    # Warmup
    print(f"   🔥 Warmup ({config.warmup_queries} queries)...")
    for i in range(config.warmup_queries):
        client.search(
            collection_name=collection_name,
            query_vector=queries[i % len(queries)].tolist(),
            limit=config.top_k
        )
    
    # Benchmark search
    print(f"   ⏱️  Running {config.search_queries} search queries...")
    latencies = []
    
    for i, query in enumerate(queries):
        start = time.perf_counter()
        
        client.search(
            collection_name=collection_name,
            query_vector=query.tolist(),
            limit=config.top_k
        )
        
        end = time.perf_counter()
        latencies.append((end - start) * 1000)
        
        if (i + 1) % 100 == 0:
            print(f"   [{(i+1)/config.search_queries*100:5.1f}%] Completed {i+1} queries")
    
    # Calculate percentiles
    latencies = np.array(latencies)
    results = {
        "database": "Qdrant",
        "vector_count": config.vector_count,
        "dimension": config.dimension,
        "queries": len(latencies),
        "latency_p50_ms": float(np.percentile(latencies, 50)),
        "latency_p95_ms": float(np.percentile(latencies, 95)),
        "latency_p99_ms": float(np.percentile(latencies, 99)),
        "latency_mean_ms": float(np.mean(latencies)),
        "latency_min_ms": float(np.min(latencies)),
        "latency_max_ms": float(np.max(latencies)),
        "insert_time_s": insert_time,
        "throughput_qps": len(latencies) / (sum(latencies) / 1000)
    }
    
    print(f"\n   📊 Qdrant Results:")
    print(f"      p50: {results['latency_p50_ms']:.2f}ms")
    print(f"      p95: {results['latency_p95_ms']:.2f}ms")
    print(f"      p99: {results['latency_p99_ms']:.2f}ms")
    print(f"      QPS: {results['throughput_qps']:.0f}")
    
    return results


# =============================================================================
# Main
# =============================================================================

def print_comparison(velesdb_results: dict, qdrant_results: dict):
    """Print comparison table."""
    print("\n" + "=" * 70)
    print("📊 BENCHMARK RESULTS COMPARISON")
    print("=" * 70)
    
    if not velesdb_results and not qdrant_results:
        print("No results to compare.")
        return
    
    headers = ["Metric", "VelesDB", "Qdrant", "Winner"]
    
    def format_row(metric: str, v_val: float, q_val: float, lower_is_better: bool = True):
        v_str = f"{v_val:.2f}" if v_val else "N/A"
        q_str = f"{q_val:.2f}" if q_val else "N/A"
        
        if v_val and q_val:
            if lower_is_better:
                winner = "🐺 VelesDB" if v_val < q_val else "🦀 Qdrant"
                ratio = q_val / v_val if v_val < q_val else v_val / q_val
            else:
                winner = "🐺 VelesDB" if v_val > q_val else "🦀 Qdrant"
                ratio = v_val / q_val if v_val > q_val else q_val / v_val
            winner += f" ({ratio:.1f}x)"
        else:
            winner = "N/A"
        
        return f"| {metric:20} | {v_str:15} | {q_str:15} | {winner:20} |"
    
    print(f"| {'Metric':20} | {'VelesDB':15} | {'Qdrant':15} | {'Winner':20} |")
    print("|" + "-" * 22 + "|" + "-" * 17 + "|" + "-" * 17 + "|" + "-" * 22 + "|")
    
    v = velesdb_results or {}
    q = qdrant_results or {}
    
    print(format_row("p50 Latency (ms)", v.get("latency_p50_ms", 0), q.get("latency_p50_ms", 0)))
    print(format_row("p95 Latency (ms)", v.get("latency_p95_ms", 0), q.get("latency_p95_ms", 0)))
    print(format_row("p99 Latency (ms)", v.get("latency_p99_ms", 0), q.get("latency_p99_ms", 0)))
    print(format_row("Throughput (QPS)", v.get("throughput_qps", 0), q.get("throughput_qps", 0), lower_is_better=False))
    print(format_row("Insert Time (s)", v.get("insert_time_s", 0), q.get("insert_time_s", 0)))
    
    print("=" * 70)


def main():
    parser = argparse.ArgumentParser(
        description="VelesDB 50M Vectors Benchmark",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
    # Quick test with 1M vectors
    python benchmark_50m.py --quick
    
    # Full 50M benchmark
    python benchmark_50m.py --full
    
    # Custom scale
    python benchmark_50m.py --vectors 10000000
    
    # VelesDB only (no Qdrant comparison)
    python benchmark_50m.py --velesdb-only

Requirements:
    - 256GB+ RAM for 50M vectors
    - pip install numpy requests qdrant-client
    - VelesDB server binary in PATH
    - Docker for Qdrant (optional)
        """
    )
    
    parser.add_argument("--quick", action="store_true", help="Quick test with 1M vectors")
    parser.add_argument("--full", action="store_true", help="Full 50M benchmark")
    parser.add_argument("--vectors", type=int, help="Custom vector count")
    parser.add_argument("--velesdb-only", action="store_true", help="Skip Qdrant comparison")
    parser.add_argument("--output", type=str, default="benchmark_results.json", help="Output JSON file")
    
    args = parser.parse_args()
    
    # Configure
    config = BenchmarkConfig()
    
    if args.quick:
        config.vector_count = 1_000_000
        config.search_queries = 100
        print("🚀 Quick mode: 1M vectors")
    elif args.full:
        config.vector_count = 50_000_000
        print("🚀 Full mode: 50M vectors")
    elif args.vectors:
        config.vector_count = args.vectors
        print(f"🚀 Custom mode: {config.vector_count:,} vectors")
    else:
        # Default to quick for safety
        config.vector_count = 1_000_000
        config.search_queries = 100
        print("🚀 Default mode: 1M vectors (use --full for 50M)")
    
    print(f"\n📋 Configuration:")
    print(f"   Vectors: {config.vector_count:,}")
    print(f"   Dimension: {config.dimension}")
    print(f"   Queries: {config.search_queries}")
    print(f"   Top-K: {config.top_k}")
    
    # Check memory
    required_gb = (config.vector_count * config.dimension * 4) / (1024**3)
    print(f"\n⚠️  Estimated RAM required: {required_gb:.1f} GB")
    
    if required_gb > 200:
        response = input("This requires significant RAM. Continue? [y/N] ")
        if response.lower() != "y":
            print("Aborted.")
            sys.exit(0)
    
    # Generate vectors
    vectors, queries = generate_vectors(config)
    
    # Run benchmarks
    velesdb_results = benchmark_velesdb(config, vectors, queries)
    
    qdrant_results = {}
    if not args.velesdb_only:
        qdrant_results = benchmark_qdrant(config, vectors, queries)
    
    # Compare
    print_comparison(velesdb_results, qdrant_results)
    
    # Save results
    results = {
        "config": {
            "vector_count": config.vector_count,
            "dimension": config.dimension,
            "queries": config.search_queries,
            "top_k": config.top_k
        },
        "velesdb": velesdb_results,
        "qdrant": qdrant_results
    }
    
    with open(args.output, "w") as f:
        json.dump(results, f, indent=2)
    
    print(f"\n💾 Results saved to {args.output}")
    print("\n🇫🇷 Benchmark by Wiscale France - contact@wiscale.fr")


if __name__ == "__main__":
    main()
