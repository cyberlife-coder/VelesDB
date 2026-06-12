"""
VelesDB + LangChain: Hybrid Dense+Sparse Search Example

Demonstrates the published ``langchain-velesdb`` connector as a
single-engine hybrid vector store for LangChain — dense (embedding),
fused dense+sparse, and vector+BM25 search in one query, without ad hoc
glue code or multiple backends.

Install:
    pip install langchain-velesdb

Replace the deterministic demo embeddings with real ones (OpenAI,
Sentence-Transformers, etc.) for production use.
"""

from __future__ import annotations

import random
import shutil

from langchain_core.embeddings import Embeddings
from langchain_velesdb import VelesDBVectorStore


class DemoEmbeddings(Embeddings):
    """Deterministic pseudo-random embeddings — replace with a real model."""

    def __init__(self, dim: int) -> None:
        self._dim = dim

    def _vector(self, text: str) -> list[float]:
        rng = random.Random(text)
        return [rng.gauss(0, 1) for _ in range(self._dim)]

    def embed_documents(self, texts: list[str]) -> list[list[float]]:
        return [self._vector(text) for text in texts]

    def embed_query(self, text: str) -> list[float]:
        return self._vector(text)


# ---------------------------------------------------------------------------
# Demo: Hybrid dense+sparse search with synthetic data
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    DIM = 128
    NUM_DOCS = 20

    documents = [
        "Retrieval-augmented generation combines search with language models",
        "Vector databases store embeddings for semantic similarity search",
        "BM25 is a classical sparse retrieval algorithm based on term frequency",
        "Hybrid search fuses dense and sparse signals for better recall",
        "VelesDB supports HNSW indexing with AVX2/AVX-512 SIMD acceleration",
        "Knowledge graphs represent relationships between entities",
        "Cosine similarity measures the angle between two vectors",
        "Product quantization compresses vectors for memory-efficient search",
        "LangChain provides abstractions for building LLM applications",
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
    ]

    # Synthetic sparse vectors (simulating BM25-style term weights)
    random.seed(42)
    sparse_vectors = [
        {random.randint(0, 9999): round(random.uniform(0.1, 3.0), 3) for _ in range(5)}
        for _ in range(NUM_DOCS)
    ]

    metadatas = [{"source": f"doc_{i}", "topic": "ai"} for i in range(NUM_DOCS)]

    DB_PATH = "./demo_velesdb_data"
    try:
        # -- Initialize store: the published connector, one dependency --
        store = VelesDBVectorStore(
            embedding=DemoEmbeddings(DIM),
            path=DB_PATH,
            collection_name="langchain_hybrid_demo",
        )

        # Insert documents with both dense embeddings and sparse vectors
        ids = store.add_texts(
            texts=documents,
            metadatas=metadatas,
            sparse_vectors=sparse_vectors,
        )
        print(f"Inserted {len(ids)} documents\n")

        # -- Dense-only search --
        print("=== Dense-Only Search ===")
        results = store.similarity_search_with_score("semantic search", k=3)
        for doc, score in results:
            print(f"  [{score:.4f}] {doc.page_content[:80]}")

        # -- Hybrid search (dense + sparse fused via RRF) --
        query_sparse = {42: 2.5, 100: 1.8, 7777: 0.9}
        print("\n=== Hybrid Search (Dense + Sparse) ===")
        results = store.similarity_search_with_score(
            "hybrid retrieval",
            k=5,
            sparse_vector=query_sparse,
        )
        for doc, score in results:
            print(f"  [{score:.4f}] {doc.page_content[:80]}")

        # -- Hybrid search (vector + BM25 full-text) --
        print("\n=== Hybrid Search (Vector + BM25) ===")
        results = store.hybrid_search(
            "dense and sparse fusion",
            k=5,
            vector_weight=0.7,  # 70% vector, 30% BM25
        )
        for doc, score in results:
            print(f"  [{score:.4f}] {doc.page_content[:80]}")

        print("\nVelesDB handles dense + sparse + fusion in a single engine.")
        print("No separate Elasticsearch, no glue code, no extra infrastructure.")
    finally:
        shutil.rmtree(DB_PATH, ignore_errors=True)
