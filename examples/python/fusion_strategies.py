#!/usr/bin/env python3
"""
Fusion Strategy Examples for VelesDB Python SDK

Demonstrates multi-query search with different fusion strategies:
- RRF (Reciprocal Rank Fusion)
- Average score
- Maximum score
- Weighted fusion

EPIC-059 US-006: Fusion strategy examples
"""

import numpy as np

# Note: VelesDB Python SDK uses PyO3 bindings
# Install: pip install velesdb-python (when published)
# For now, build from source: maturin develop


def generate_embedding(seed: int, dim: int = 128) -> list[float]:
    """Generate a deterministic mock embedding for demo purposes."""
    np.random.seed(seed)
    vec = np.random.randn(dim).astype(np.float32)
    vec = vec / np.linalg.norm(vec)
    return vec.tolist()


def example_rrf_fusion():
    """
    RRF (Reciprocal Rank Fusion)
    
    Best for: Multi-query generation (MQG) pipelines
    Formula: score = sum(1 / (k + rank)) for each query
    """
    print("\n=== RRF Fusion (Reciprocal Rank Fusion) ===")
    print("Best for: Combining results from multiple query reformulations")
    print()
    
    # Generate multiple query variants (like MQG/HyDE)
    queries = [
        generate_embedding(42),   # Original query
        generate_embedding(43),   # Reformulation 1
        generate_embedding(44),   # Reformulation 2
    ]
    
    print("Python SDK:")
    print("  from velesdb import FusionStrategy")
    print()
    print("  queries = [original_query, reformulation1, reformulation2]")
    print("  results = collection.multi_query_search(")
    print("      vectors=queries,")
    print("      top_k=10,")
    print("      fusion=FusionStrategy.rrf(k=60)  # k=60 is standard")
    print("  )")
    print()
    print("CLI:")
    print('  velesdb multi-search ./data my_collection \\')
    print('    \'[[0.1, 0.2, ...], [0.15, 0.25, ...], [0.12, 0.22, ...]]\' \\')
    print('    --strategy rrf --rrf-k 60 -k 10')


def example_average_fusion():
    """
    Average Score Fusion
    
    Best for: Uniform query importance
    Formula: score = avg(scores)
    """
    print("\n=== Average Score Fusion ===")
    print("Best for: When all query variants have equal importance")
    print()
    
    print("Python SDK:")
    print("  results = collection.multi_query_search(")
    print("      vectors=queries,")
    print("      top_k=10,")
    print("      fusion=FusionStrategy.average()")
    print("  )")
    print()
    print("CLI:")
    print('  velesdb multi-search ./data my_collection \\')
    print('    \'[[...], [...]]\' --strategy average -k 10')


def example_maximum_fusion():
    """
    Maximum Score Fusion
    
    Best for: When any strong match is important
    Formula: score = max(scores)
    """
    print("\n=== Maximum Score Fusion ===")
    print("Best for: Finding best match across any query variant")
    print()
    
    print("Python SDK:")
    print("  results = collection.multi_query_search(")
    print("      vectors=queries,")
    print("      top_k=10,")
    print("      fusion=FusionStrategy.maximum()")
    print("  )")
    print()
    print("CLI:")
    print('  velesdb multi-search ./data my_collection \\')
    print('    \'[[...], [...]]\' --strategy max -k 10')


def example_weighted_fusion():
    """
    Weighted Fusion
    
    Best for: Custom scoring with avg, max, and hit count
    Formula: score = avg_weight*avg + max_weight*max + hit_weight*hit_ratio
    """
    print("\n=== Weighted Fusion ===")
    print("Best for: Fine-tuned relevance scoring")
    print()
    
    print("Python SDK:")
    print("  results = collection.multi_query_search(")
    print("      vectors=queries,")
    print("      top_k=10,")
    print("      fusion=FusionStrategy.weighted(")
    print("          avg_weight=0.5,   # Average score weight")
    print("          max_weight=0.3,   # Max score weight")
    print("          hit_weight=0.2    # Hit ratio weight")
    print("      )")
    print("  )")
    print()
    print("CLI:")
    print('  velesdb multi-search ./data my_collection \\')
    print('    \'[[...], [...]]\' --strategy weighted -k 10')


def example_hybrid_with_fusion():
    """
    Hybrid Search + Multi-Query Fusion
    
    Combines vector + text search with multi-query fusion.
    """
    print("\n=== Hybrid Search + Multi-Query Fusion ===")
    print("Best for: RAG pipelines with query expansion")
    print()
    
    print("Python SDK (combining hybrid with multi-query):")
    print("  # Step 1: Generate query variants")
    print("  original = embed('How does photosynthesis work?')")
    print("  expanded = embed('photosynthesis process plants energy light')")
    print("  hyde = embed(llm_generate_hypothetical_answer(query))")
    print()
    print("  # Step 2: Multi-query vector search")
    print("  vector_results = collection.multi_query_search(")
    print("      vectors=[original, expanded, hyde],")
    print("      top_k=20,")
    print("      fusion=FusionStrategy.rrf(k=60)")
    print("  )")
    print()
    print("  # Step 3: Combine with BM25 text search")
    print("  text_results = collection.text_search('photosynthesis', top_k=20)")
    print()
    print("  # Step 4: Final RRF fusion")
    print("  final = rrf_merge(vector_results, text_results, k=60)")


def main():
    """Run all fusion strategy examples."""
    print("=" * 60)
    print("VelesDB Fusion Strategy Examples - Python SDK")
    print("=" * 60)
    
    example_rrf_fusion()
    example_average_fusion()
    example_maximum_fusion()
    example_weighted_fusion()
    example_hybrid_with_fusion()
    
    print("\n" + "=" * 60)
    print("Fusion strategies are key for Multi-Query Generation (MQG)")
    print("See: docs/guides/MULTI_QUERY_SEARCH.md")
    print("=" * 60)


if __name__ == "__main__":
    main()
