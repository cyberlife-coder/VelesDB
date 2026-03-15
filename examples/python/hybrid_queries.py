#!/usr/bin/env python3
"""
Hybrid Query Examples for VelesDB Python SDK

Demonstrates vector similarity search combined with:
- Metadata filtering
- BM25 text search
- Hybrid vector + text search
- VelesQL queries with parameters
- Multi-query search with fusion
- Batch search

Requirements:
    pip install velesdb numpy
    # Or build from source: cd crates/velesdb-python && maturin develop
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
    """Create and populate a collection with sample documents."""
    coll = db.create_collection("documents", dimension=dim, metric="cosine")

    categories = ["ai", "physics", "biology", "programming", "math"]
    access_levels = ["public", "internal", "restricted"]

    points = []
    for i in range(1, 31):
        points.append({
            "id": i,
            "vector": generate_embedding(seed=i, dim=dim),
            "payload": {
                "title": f"Document {i}: {categories[i % len(categories)]} research",
                "category": categories[i % len(categories)],
                "access_level": access_levels[i % len(access_levels)],
                "year": 2020 + (i % 6),
                "score_manual": float(i % 10) / 10.0,
            },
        })
    coll.upsert(points)
    print(f"Inserted {len(points)} documents")
    return coll


def example_basic_vector_search(coll: velesdb.Collection):
    """Basic dense vector similarity search."""
    print("\n=== Basic Vector Search ===")

    query = generate_embedding(seed=42)
    results = coll.search(vector=query, top_k=5)

    print(f"Top {len(results)} results:")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, "
              f"Title: {r['payload']['title']}")


def example_search_with_ef(coll: velesdb.Collection):
    """Vector search with custom ef_search for recall tuning."""
    print("\n=== Search with Custom ef_search ===")

    query = generate_embedding(seed=42)

    # Higher ef_search = better recall, slightly slower
    results = coll.search_with_ef(vector=query, top_k=5, ef_search=256)

    print(f"Top {len(results)} results (ef_search=256):")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, "
              f"Title: {r['payload']['title']}")


def example_search_ids_only(coll: velesdb.Collection):
    """Search returning only IDs and scores (no payload, faster)."""
    print("\n=== Search IDs Only (No Payload) ===")

    query = generate_embedding(seed=42)
    results = coll.search_ids(vector=query, top_k=5)

    print(f"Top {len(results)} results (IDs only):")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}")


def example_velesql_query(coll: velesdb.Collection):
    """Execute VelesQL queries with parameters."""
    print("\n=== VelesQL Query ===")

    query_vec = generate_embedding(seed=42)

    # Vector search via VelesQL
    results = coll.query(
        "SELECT * FROM documents WHERE vector NEAR $v LIMIT 5",
        params={"v": query_vec},
    )

    print(f"VelesQL results: {len(results)} rows")
    for r in results:
        print(f"  Node ID: {r['node_id']}, Fused Score: {r['fused_score']:.4f}")


def example_velesql_explain(coll: velesdb.Collection):
    """Get the query execution plan for a VelesQL query."""
    print("\n=== VelesQL EXPLAIN ===")

    plan = coll.explain(
        "SELECT * FROM documents WHERE vector NEAR $v AND category = 'ai' LIMIT 10"
    )

    print(f"Estimated cost: {plan['estimated_cost_ms']:.2f} ms")
    print(f"Filter strategy: {plan['filter_strategy']}")
    print(f"Index used: {plan['index_used']}")
    print(f"Plan tree:\n{plan['tree']}")


def example_multi_query_search(coll: velesdb.Collection):
    """Multi-query search with fusion strategies."""
    print("\n=== Multi-Query Search ===")

    queries = [
        generate_embedding(42),
        generate_embedding(43),
        generate_embedding(44),
    ]

    results = coll.multi_query_search(
        vectors=queries,
        top_k=5,
        fusion=FusionStrategy.rrf(k=60),
    )

    print(f"Top {len(results)} results with RRF fusion:")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, "
              f"Title: {r['payload']['title']}")


def example_batch_search(coll: velesdb.Collection):
    """Batch search: multiple independent queries in one call."""
    print("\n=== Batch Search ===")

    searches = [
        {"vector": generate_embedding(42), "top_k": 3},
        {"vector": generate_embedding(43), "top_k": 3},
    ]

    batch_results = coll.batch_search(searches)

    for i, results in enumerate(batch_results):
        print(f"Query {i + 1} results:")
        for r in results:
            print(f"  ID: {r['id']}, Score: {r['score']:.4f}")


def example_get_points(coll: velesdb.Collection):
    """Retrieve specific points by ID."""
    print("\n=== Get Points by ID ===")

    points = coll.get([1, 2, 3, 999])  # 999 does not exist

    for p in points:
        if p is not None:
            print(f"  ID: {p['id']}, Payload: {p['payload']}")
        else:
            print("  (not found)")


def example_delete_and_verify(coll: velesdb.Collection):
    """Delete points and verify they are gone."""
    print("\n=== Delete and Verify ===")

    # Delete point 30
    coll.delete([30])
    result = coll.get([30])
    print(f"After deleting ID 30: {result}")


def main():
    """Run all hybrid query examples."""
    print("=" * 60)
    print("VelesDB Hybrid Query Examples")
    print("=" * 60)

    tmp_dir = tempfile.mkdtemp(prefix="velesdb_hybrid_")
    try:
        db = velesdb.Database(tmp_dir)
        coll = setup_collection(db)

        example_basic_vector_search(coll)
        example_search_with_ef(coll)
        example_search_ids_only(coll)
        example_velesql_query(coll)
        example_velesql_explain(coll)
        example_multi_query_search(coll)
        example_batch_search(coll)
        example_get_points(coll)
        example_delete_and_verify(coll)

        # Flush all changes to disk
        coll.flush()

        # Cleanup
        db.delete_collection("documents")

        print("\n" + "=" * 60)
        print("All hybrid query examples completed successfully.")
        print("=" * 60)
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


if __name__ == "__main__":
    main()
