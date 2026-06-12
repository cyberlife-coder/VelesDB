"""
VelesDB + LlamaIndex: Hybrid Search with Product Quantization Example

Demonstrates the published ``llama-index-vector-stores-velesdb`` connector
as a single-engine hybrid vector store for LlamaIndex — dense (embedding),
fused dense+sparse, and vector+BM25 search, with optional Product
Quantization (PQ) for memory-efficient storage.

Install:
    pip install llama-index-vector-stores-velesdb

Replace the deterministic demo vectors with real embeddings (OpenAI,
Sentence-Transformers, etc.) for production use.
"""

from __future__ import annotations

import random
import shutil

from llama_index.core.schema import TextNode
from llama_index.core.vector_stores.types import VectorStoreQuery
from llamaindex_velesdb import VelesDBVectorStore


def _print_result(result) -> None:
    """Print nodes and scores from a VectorStoreQueryResult."""
    for node, score in zip(result.nodes, result.similarities):
        print(f"  [{score:.4f}] {node.get_content()[:80]}")


# ---------------------------------------------------------------------------
# Demo: Hybrid search + Product Quantization with synthetic data
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    DIM = 128
    NUM_DOCS = 50  # More docs to make PQ training meaningful

    documents = [
        "Retrieval-augmented generation combines search with language models",
        "Vector databases store embeddings for semantic similarity search",
        "BM25 is a classical sparse retrieval algorithm based on term frequency",
        "Hybrid search fuses dense and sparse signals for better recall",
        "VelesDB supports HNSW indexing with AVX2/AVX-512 SIMD acceleration",
        "Knowledge graphs represent relationships between entities",
        "Cosine similarity measures the angle between two vectors",
        "Product quantization compresses vectors for memory-efficient search",
        "LlamaIndex provides a framework for LLM data applications",
        "Sparse vectors encode term importance weights for keyword matching",
        "Graph traversal algorithms like BFS explore connected nodes",
        "Embedding models convert text into fixed-dimensional vectors",
        "Approximate nearest neighbor search trades exactness for speed",
        "VelesDB combines vector, graph, and column store in one engine",
        "Sentence transformers produce high-quality text embeddings",
        "Inverted indexes map terms to documents for fast lookup",
        "Sub-millisecond latency is critical for real-time AI applications",
        "Named sparse vectors allow multiple sparse indexes per collection",
        "Reciprocal rank fusion merges ranked lists from different sources",
        "Offline-first databases work without cloud connectivity",
        "PQ divides vectors into sub-spaces and quantizes each independently",
        "OPQ applies an orthogonal rotation before quantization for better quality",
        "Binary quantization provides 32x compression with single-bit encoding",
        "HNSW builds a multi-layer navigable small world graph for fast search",
        "Agent memory patterns include semantic, episodic, and procedural types",
        "VelesQL extends SQL with vector similarity and graph traversal syntax",
        "Column stores organize data by columns for efficient analytical queries",
        "WAL ensures durability by logging writes before applying them",
        "Memory-mapped files enable efficient I/O for large vector collections",
        "Sharded storage distributes data across multiple files for parallelism",
        "Distance metrics include cosine, euclidean, and dot product",
        "k-means clustering groups vectors by proximity to centroids",
        "Dimension reduction techniques like PCA preserve important variance",
        "Batch processing amortizes overhead across multiple operations",
        "Stream ingestion handles continuous data flows with backpressure",
        "Query plan caching avoids redundant optimization for repeated queries",
        "Filter pushdown evaluates predicates before expensive similarity search",
        "Multi-tenancy isolates data between different users or applications",
        "Schema migration tools handle evolving data models gracefully",
        "Benchmarking with Criterion provides statistically rigorous measurements",
        "SIMD intrinsics accelerate distance computations on modern CPUs",
        "Lock-free data structures reduce contention in concurrent systems",
        "Compaction merges small files into larger ones for read efficiency",
        "Scoring functions combine multiple signals into a unified ranking",
        "Top-k selection algorithms efficiently find highest-scoring items",
        "Payload filtering narrows search results by metadata predicates",
        "Index building is an offline process that prepares data for search",
        "Query expansion adds related terms to improve recall",
        "Re-ranking refines initial retrieval results with a more precise model",
        "End-to-end evaluation measures system quality on real-world tasks",
    ]

    # Synthetic dense embeddings (replace with real model in production)
    random.seed(42)
    dense_vectors = [
        [random.gauss(0, 1) for _ in range(DIM)] for _ in range(NUM_DOCS)
    ]

    # Synthetic sparse vectors (simulating BM25-style term weights)
    sparse_vectors = [
        {random.randint(0, 9999): round(random.uniform(0.1, 3.0), 3) for _ in range(5)}
        for _ in range(NUM_DOCS)
    ]

    DB_PATH = "./demo_velesdb_data"
    try:
        # -- Initialize store: the published connector, one dependency --
        store = VelesDBVectorStore(
            path=DB_PATH,
            collection_name="llamaindex_hybrid_demo",
        )

        # Build LlamaIndex TextNodes with embeddings
        nodes = [
            TextNode(
                text=text,
                metadata={"source": f"doc_{i}", "topic": "ai"},
                id_=f"node_{i}",
                embedding=embedding,
            )
            for i, (text, embedding) in enumerate(zip(documents, dense_vectors))
        ]

        # Insert nodes with sparse vectors
        ids = store.add(nodes, sparse_vectors=sparse_vectors)
        print(f"Inserted {len(ids)} nodes\n")

        # -- Train Product Quantization --
        # PQ compresses vectors for ~8x memory reduction.
        # Requires sufficient vectors for meaningful centroid training.
        print("=== Training Product Quantization ===")
        try:
            status = store.train_pq(m=8, k=256)
            print(f"  PQ training: {status}")
        except Exception as e:
            print(f"  PQ training skipped (expected with small dataset): {e}")

        # -- Dense-only search --
        query_dense = [random.gauss(0, 1) for _ in range(DIM)]
        print("\n=== Dense-Only Search ===")
        result = store.query(
            VectorStoreQuery(query_embedding=query_dense, similarity_top_k=3)
        )
        _print_result(result)

        # -- Hybrid search (dense + sparse fused via RRF) --
        query_sparse = {42: 2.5, 100: 1.8, 7777: 0.9}
        print("\n=== Hybrid Search (Dense + Sparse) ===")
        result = store.query(
            VectorStoreQuery(query_embedding=query_dense, similarity_top_k=5),
            sparse_vector=query_sparse,
        )
        _print_result(result)

        # -- Hybrid search (vector + BM25 full-text) --
        print("\n=== Hybrid Search (Vector + BM25) ===")
        result = store.hybrid_query(
            "hybrid retrieval with rank fusion",
            query_embedding=query_dense,
            similarity_top_k=5,
            vector_weight=0.7,  # 70% vector, 30% BM25
        )
        _print_result(result)

        print("\nVelesDB handles dense + sparse + PQ compression in a single engine.")
        print("No separate systems needed for hybrid search or quantization.")
    finally:
        shutil.rmtree(DB_PATH, ignore_errors=True)
