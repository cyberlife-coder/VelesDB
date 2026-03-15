#!/usr/bin/env python3
"""
Fusion Strategy Examples for VelesDB Python SDK

Demonstrates multi-query search with different fusion strategies:
- RRF (Reciprocal Rank Fusion)
- Average score
- Maximum score
- Weighted fusion
- Relative Score Fusion

Requirements:
    pip install velesdb numpy
    # Or build from source: cd crates/velesdb-python && maturin develop

EPIC-059 US-006: Fusion strategy examples
"""

import shutil
import tempfile

import numpy as np

import velesdb
from velesdb import FusionStrategy


def generate_embedding(seed: int, dim: int = 128) -> list[float]:
    """Generate a deterministic normalized embedding for demo purposes."""
    rng = np.random.RandomState(seed)
    vec = rng.randn(dim).astype(np.float32)
    vec = vec / np.linalg.norm(vec)
    return vec.tolist()


def setup_collection(db: velesdb.Database, dim: int = 128) -> velesdb.Collection:
    """Create a collection and populate it with sample vectors."""
    coll = db.create_collection("articles", dimension=dim, metric="cosine")

    # Insert 50 sample documents with deterministic embeddings
    points = []
    categories = ["science", "technology", "health", "politics", "sports"]
    for i in range(1, 51):
        points.append({
            "id": i,
            "vector": generate_embedding(seed=i, dim=dim),
            "payload": {
                "title": f"Article {i}",
                "category": categories[i % len(categories)],
            },
        })
    coll.upsert(points)
    print(f"Inserted {len(points)} articles into collection 'articles'")
    return coll


def example_rrf_fusion(coll: velesdb.Collection):
    """
    RRF (Reciprocal Rank Fusion)

    Best for: Multi-query generation (MQG) pipelines.
    Formula: score = sum(1 / (k + rank)) for each query.
    """
    print("\n=== RRF Fusion (Reciprocal Rank Fusion) ===")

    # Simulate multiple query variants (e.g., original + reformulations)
    queries = [
        generate_embedding(42),   # Original query
        generate_embedding(43),   # Reformulation 1
        generate_embedding(44),   # Reformulation 2
    ]

    results = coll.multi_query_search(
        vectors=queries,
        top_k=5,
        fusion=FusionStrategy.rrf(k=60),
    )

    print(f"Top {len(results)} results with RRF (k=60):")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, "
              f"Title: {r['payload']['title']}")


def example_average_fusion(coll: velesdb.Collection):
    """
    Average Score Fusion

    Best for: Uniform query importance.
    Formula: score = mean(scores across all queries).
    """
    print("\n=== Average Score Fusion ===")

    queries = [
        generate_embedding(42),
        generate_embedding(43),
    ]

    results = coll.multi_query_search(
        vectors=queries,
        top_k=5,
        fusion=FusionStrategy.average(),
    )

    print(f"Top {len(results)} results with Average fusion:")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, "
              f"Title: {r['payload']['title']}")


def example_maximum_fusion(coll: velesdb.Collection):
    """
    Maximum Score Fusion

    Best for: When any single strong match matters.
    Formula: score = max(scores across all queries).
    """
    print("\n=== Maximum Score Fusion ===")

    queries = [
        generate_embedding(42),
        generate_embedding(43),
    ]

    results = coll.multi_query_search(
        vectors=queries,
        top_k=5,
        fusion=FusionStrategy.maximum(),
    )

    print(f"Top {len(results)} results with Maximum fusion:")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, "
              f"Title: {r['payload']['title']}")


def example_weighted_fusion(coll: velesdb.Collection):
    """
    Weighted Fusion

    Best for: Custom scoring with control over avg, max, and hit count.
    Formula: score = avg_weight * avg + max_weight * max + hit_weight * hit_ratio
    Weights must sum to 1.0.
    """
    print("\n=== Weighted Fusion ===")

    queries = [
        generate_embedding(42),
        generate_embedding(43),
        generate_embedding(44),
    ]

    results = coll.multi_query_search(
        vectors=queries,
        top_k=5,
        fusion=FusionStrategy.weighted(
            avg_weight=0.5,
            max_weight=0.3,
            hit_weight=0.2,
        ),
    )

    print(f"Top {len(results)} results with Weighted fusion:")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, "
              f"Title: {r['payload']['title']}")


def example_relative_score_fusion(coll: velesdb.Collection):
    """
    Relative Score Fusion (RSF)

    Best for: Hybrid dense + sparse search with explicit weight control.
    Linearly combines dense and sparse scores.
    """
    print("\n=== Relative Score Fusion ===")

    # RSF is designed for hybrid dense+sparse pipelines.
    # Here we demonstrate the strategy creation and a dense-only search
    # to show the API. For true hybrid use, provide sparse_vector as well.
    strategy = FusionStrategy.relative_score(dense_weight=0.7, sparse_weight=0.3)
    print(f"Created strategy: {strategy}")

    # Dense-only multi-query search still works with any strategy
    queries = [generate_embedding(42), generate_embedding(43)]
    results = coll.multi_query_search(
        vectors=queries,
        top_k=5,
        fusion=FusionStrategy.rrf(),  # Use RRF for dense-only multi-query
    )

    print(f"Top {len(results)} results:")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, "
              f"Title: {r['payload']['title']}")


def main():
    """Run all fusion strategy examples."""
    print("=" * 60)
    print("VelesDB Fusion Strategy Examples")
    print("=" * 60)

    # Use a temporary directory for demo data
    tmp_dir = tempfile.mkdtemp(prefix="velesdb_fusion_")
    try:
        db = velesdb.Database(tmp_dir)
        coll = setup_collection(db)

        example_rrf_fusion(coll)
        example_average_fusion(coll)
        example_maximum_fusion(coll)
        example_weighted_fusion(coll)
        example_relative_score_fusion(coll)

        # Cleanup
        db.delete_collection("articles")
        print("\n" + "=" * 60)
        print("All fusion strategy examples completed successfully.")
        print("=" * 60)
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


if __name__ == "__main__":
    main()
