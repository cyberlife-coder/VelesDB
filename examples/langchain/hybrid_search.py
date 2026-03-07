"""
VelesDB + LangChain: Hybrid Dense+Sparse Search Example

Demonstrates VelesDB as a single-engine hybrid vector store for LangChain,
combining dense (embedding) and sparse (BM25-style) search in one query
without external glue code or multiple backends.

Replace random vectors with real embeddings (OpenAI, Sentence-Transformers, etc.)
for production use.
"""

from __future__ import annotations

import random
import uuid
from typing import Any, Iterable, Optional

import velesdb
from langchain_core.documents import Document
from langchain_core.embeddings import Embeddings
from langchain_core.vectorstores import VectorStore


class VelesDBVectorStore(VectorStore):
    """LangChain VectorStore backed by VelesDB with hybrid dense+sparse search.

    VelesDB handles both dense vector similarity and sparse keyword matching
    in a single engine, eliminating the need to combine separate systems.
    """

    def __init__(
        self,
        collection_name: str,
        db_path: str = "./velesdb_data",
        dimension: int = 768,
        metric: str = "cosine",
        embedding_function: Optional[Embeddings] = None,
    ) -> None:
        self._db = velesdb.Database(db_path)
        self._collection = self._db.get_or_create_collection(
            collection_name, dimension=dimension, metric=metric
        )
        self._embedding_function = embedding_function
        self._dimension = dimension

    @property
    def embeddings(self) -> Optional[Embeddings]:
        """Return the embedding function, if set."""
        return self._embedding_function

    def add_texts(
        self,
        texts: Iterable[str],
        metadatas: Optional[list[dict]] = None,
        embeddings: Optional[list[list[float]]] = None,
        sparse_vectors: Optional[list[dict[int, float]]] = None,
        ids: Optional[list[str]] = None,
        **kwargs: Any,
    ) -> list[str]:
        """Add texts with optional dense embeddings and sparse vectors.

        Args:
            texts: Documents to insert.
            metadatas: Per-document metadata dicts.
            embeddings: Pre-computed dense vectors. If None and an embedding
                function is set, embeddings are generated automatically.
            sparse_vectors: Optional sparse vectors for hybrid search.
                Each is a dict mapping dimension index to weight.
            ids: Document IDs. Generated if not provided.

        Returns:
            List of document IDs.
        """
        text_list = list(texts)

        # Generate embeddings if not provided
        if embeddings is None and self._embedding_function is not None:
            embeddings = self._embedding_function.embed_documents(text_list)

        if embeddings is not None and len(embeddings) != len(text_list):
            raise ValueError(
                f"Number of embeddings ({len(embeddings)}) does not match "
                f"number of texts ({len(text_list)})"
            )

        # Build point dicts
        points = []
        result_ids = []
        for i, text in enumerate(text_list):
            doc_id = ids[i] if ids else str(uuid.uuid4())
            result_ids.append(doc_id)

            payload = {"text": text}
            if metadatas and i < len(metadatas):
                payload.update(metadatas[i])

            point: dict[str, Any] = {
                "id": doc_id,
                "payload": payload,
            }

            if embeddings is not None:
                point["vector"] = embeddings[i]

            if sparse_vectors is not None and i < len(sparse_vectors):
                point["sparse_vector"] = sparse_vectors[i]

            points.append(point)

        self._collection.upsert(points)
        return result_ids

    def similarity_search(
        self,
        query: str,
        k: int = 4,
        **kwargs: Any,
    ) -> list[Document]:
        """Search for documents similar to a query.

        Supports dense-only, sparse-only, or hybrid search depending on
        which vectors are provided in kwargs.

        Args:
            query: Query text (used with embedding function if available).
            k: Number of results to return.
            **kwargs: Optional ``query_embedding``, ``embedding``, or
                ``sparse_vector`` for hybrid search.

        Returns:
            List of matching LangChain Document objects.
        """
        results_with_scores = self.similarity_search_with_score(query, k, **kwargs)
        return [doc for doc, _score in results_with_scores]

    def similarity_search_with_score(
        self,
        query: str,
        k: int = 4,
        **kwargs: Any,
    ) -> list[tuple[Document, float]]:
        """Search with relevance scores.

        Args:
            query: Query text.
            k: Number of results.
            **kwargs: Optional ``query_embedding``/``embedding`` and
                ``sparse_vector``.

        Returns:
            List of (Document, score) tuples, highest score first.
        """
        # Resolve dense query embedding
        query_embedding = kwargs.get("query_embedding") or kwargs.get("embedding")
        if query_embedding is None and self._embedding_function is not None:
            query_embedding = self._embedding_function.embed_query(query)

        sparse_vector = kwargs.get("sparse_vector")

        results = self._collection.search(
            vector=query_embedding,
            sparse_vector=sparse_vector,
            top_k=k,
        )

        docs_and_scores: list[tuple[Document, float]] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.pop("text", "")
            score = result.get("score", 0.0)
            doc = Document(page_content=text, metadata=payload)
            docs_and_scores.append((doc, score))

        return docs_and_scores

    @classmethod
    def from_texts(
        cls,
        texts: list[str],
        embedding: Optional[Embeddings] = None,
        metadatas: Optional[list[dict]] = None,
        collection_name: str = "langchain_collection",
        db_path: str = "./velesdb_data",
        dimension: int = 768,
        metric: str = "cosine",
        **kwargs: Any,
    ) -> "VelesDBVectorStore":
        """Create a VelesDBVectorStore from a list of texts.

        Args:
            texts: Documents to insert.
            embedding: Embedding function for generating vectors.
            metadatas: Per-document metadata.
            collection_name: Name of the VelesDB collection.
            db_path: Path to VelesDB data directory.
            dimension: Vector dimensionality.
            metric: Distance metric (cosine, euclidean, dot).

        Returns:
            Initialized VelesDBVectorStore with documents inserted.
        """
        store = cls(
            collection_name=collection_name,
            db_path=db_path,
            dimension=dimension,
            metric=metric,
            embedding_function=embedding,
        )
        store.add_texts(texts, metadatas=metadatas, **kwargs)
        return store


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

    metadatas = [{"source": f"doc_{i}", "topic": "ai"} for i in range(NUM_DOCS)]

    # -- Initialize store --
    store = VelesDBVectorStore(
        collection_name="langchain_hybrid_demo",
        db_path="./demo_velesdb_data",
        dimension=DIM,
    )

    # Insert documents with both dense and sparse vectors
    ids = store.add_texts(
        texts=documents,
        embeddings=dense_vectors,
        sparse_vectors=sparse_vectors,
        metadatas=metadatas,
    )
    print(f"Inserted {len(ids)} documents\n")

    # -- Dense-only search --
    query_dense = [random.gauss(0, 1) for _ in range(DIM)]
    print("=== Dense-Only Search ===")
    results = store.similarity_search_with_score(
        query="semantic search",
        k=3,
        query_embedding=query_dense,
    )
    for doc, score in results:
        print(f"  [{score:.4f}] {doc.page_content[:80]}")

    # -- Sparse-only search --
    query_sparse = {42: 2.5, 100: 1.8, 7777: 0.9}
    print("\n=== Sparse-Only Search ===")
    results = store.similarity_search_with_score(
        query="keyword matching",
        k=3,
        sparse_vector=query_sparse,
    )
    for doc, score in results:
        print(f"  [{score:.4f}] {doc.page_content[:80]}")

    # -- Hybrid search (dense + sparse fused via RRF) --
    print("\n=== Hybrid Search (Dense + Sparse) ===")
    results = store.similarity_search_with_score(
        query="hybrid retrieval",
        k=5,
        query_embedding=query_dense,
        sparse_vector=query_sparse,
    )
    for doc, score in results:
        print(f"  [{score:.4f}] {doc.page_content[:80]}")

    print("\nVelesDB handles dense + sparse + fusion in a single engine.")
    print("No separate Elasticsearch, no glue code, no extra infrastructure.")
