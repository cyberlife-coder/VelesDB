"""
VelesDB + LlamaIndex: Hybrid Search with Product Quantization Example

Demonstrates VelesDB as a single-engine hybrid vector store for LlamaIndex,
combining dense (embedding) and sparse (BM25-style) search with optional
Product Quantization (PQ) for memory-efficient storage.

Replace random vectors with real embeddings (OpenAI, Sentence-Transformers, etc.)
for production use.
"""

from __future__ import annotations

import hashlib
import random
import shutil
import uuid
from typing import Any

import velesdb
from llama_index.core.schema import NodeWithScore, TextNode
from llama_index.core.vector_stores.types import (
    BasePydanticVectorStore,
    VectorStoreQuery,
    VectorStoreQueryResult,
)


def _str_to_u64(s: str) -> int:
    """Convert a string to a deterministic u64 ID for VelesDB.

    VelesDB's Rust binding expects ``id`` fields as ``u64``.  This helper
    hashes the input string with SHA-256 and takes the first 8 bytes as a
    little-endian unsigned integer, giving a collision-resistant mapping that
    fits in a ``u64``.
    """
    return int.from_bytes(hashlib.sha256(s.encode()).digest()[:8], "little")


class VelesDBVectorStore(BasePydanticVectorStore):
    """LlamaIndex VectorStore backed by VelesDB with hybrid search and PQ support.

    VelesDB handles dense + sparse + fusion in a single engine and supports
    Product Quantization for ~8x memory reduction on large collections.
    """

    stores_text: bool = True
    flat_metadata: bool = True

    collection_name: str = "llamaindex_collection"
    db_path: str = "./velesdb_data"
    dimension: int = 768
    metric: str = "cosine"

    # Private attributes (not part of the Pydantic model)
    _db: Any = None
    _collection: Any = None

    def __init__(
        self,
        collection_name: str = "llamaindex_collection",
        db_path: str = "./velesdb_data",
        dimension: int = 768,
        metric: str = "cosine",
        **kwargs: Any,
    ) -> None:
        super().__init__(
            collection_name=collection_name,
            db_path=db_path,
            dimension=dimension,
            metric=metric,
            **kwargs,
        )
        self._db = velesdb.Database(db_path)
        self._collection = self._db.get_or_create_collection(
            collection_name, dimension=dimension, metric=metric
        )

    @property
    def client(self) -> Any:
        """Return the underlying VelesDB Database instance."""
        return self._db

    def add(self, nodes: list[TextNode], **kwargs: Any) -> list[str]:
        """Add nodes to the vector store.

        Extracts embeddings, text, metadata, and optional sparse vectors
        from LlamaIndex nodes and upserts them into VelesDB.

        Args:
            nodes: LlamaIndex TextNode objects with embeddings.
            **kwargs: Optional ``sparse_vectors`` list of dicts mapping
                dimension index to weight.

        Returns:
            List of original node IDs (strings).  The original string ID is
            stored in the payload under ``_node_id`` so it can be recovered
            from query results via :meth:`query`.
        """
        sparse_vectors = kwargs.get("sparse_vectors")
        points = []
        result_ids = []

        for i, node in enumerate(nodes):
            # Keep the original string node ID for the caller.
            node_id = node.node_id or str(uuid.uuid4())
            result_ids.append(node_id)

            # VelesDB requires u64 point IDs — hash the string ID.
            point_id = _str_to_u64(node_id)

            payload: dict[str, Any] = {
                "text": node.get_content(),
                "_node_id": node_id,
            }
            if node.metadata:
                payload.update(node.metadata)

            point: dict[str, Any] = {
                "id": point_id,
                "payload": payload,
            }

            if node.embedding is not None:
                point["vector"] = node.embedding

            if sparse_vectors is not None and i < len(sparse_vectors):
                point["sparse_vector"] = sparse_vectors[i]

            points.append(point)

        self._collection.upsert(points)
        return result_ids

    def delete(self, ref_doc_id: str, **kwargs: Any) -> None:
        """Delete a document by reference ID.

        Args:
            ref_doc_id: The document ID to delete.
        """
        # VelesDB deletion is handled at the collection level
        raise NotImplementedError(
            "Delete is not yet supported in this example. "
            "Use the VelesDB Python SDK directly for deletion."
        )

    def query(self, query: VectorStoreQuery, **kwargs: Any) -> VectorStoreQueryResult:
        """Query the vector store with optional hybrid search.

        Supports dense-only, sparse-only, or hybrid (dense+sparse) queries
        depending on which vectors are provided.

        Args:
            query: LlamaIndex VectorStoreQuery with query_embedding.
            **kwargs: Optional ``sparse_vector`` dict for hybrid search.

        Returns:
            VectorStoreQueryResult with matching nodes and scores.
        """
        sparse_vector = kwargs.get("sparse_vector")

        results = self._collection.search_request(
            velesdb.SearchOptions(
                vector=query.query_embedding,
                sparse_vector=sparse_vector,
                top_k=query.similarity_top_k,
            )
        )

        nodes: list[TextNode] = []
        similarities: list[float] = []
        ids: list[str] = []

        for result in results:
            payload = result.get("payload", {})
            text = payload.pop("text", "")
            score = result.get("score", 0.0)
            # Recover the original string node ID stored by add(); fall back
            # to the numeric u64 if the payload entry is missing.
            node_id = payload.pop("_node_id", str(result.get("id", "")))

            node = TextNode(text=text, metadata=payload, id_=node_id)
            nodes.append(node)
            similarities.append(score)
            ids.append(node_id)

        return VectorStoreQueryResult(
            nodes=[NodeWithScore(node=n, score=s) for n, s in zip(nodes, similarities)],
            similarities=similarities,
            ids=ids,
        )

    def train_pq(self, m: int = 8, k: int = 256, opq: bool = False) -> str:
        """Train Product Quantization on the collection.

        PQ compresses vectors for ~8x memory reduction while maintaining
        high recall. Requires enough vectors in the collection for
        meaningful cluster training.

        Args:
            m: Number of PQ sub-quantizers (splits vector into m sub-spaces).
            k: Number of centroids per sub-quantizer (typically 256).
            opq: Enable Optimized PQ with rotation for better quality.

        Returns:
            Training status message.
        """
        return self._db.train_pq(self.collection_name, m=m, k=k, opq=opq)


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
        # -- Initialize store --
        store = VelesDBVectorStore(
            collection_name="llamaindex_hybrid_demo",
            db_path=DB_PATH,
            dimension=DIM,
        )

        # Build LlamaIndex TextNodes with embeddings
        nodes = []
        for i, (text, embedding) in enumerate(zip(documents, dense_vectors)):
            node = TextNode(
                text=text,
                metadata={"source": f"doc_{i}", "topic": "ai"},
                id_=f"node_{i}",
                embedding=embedding,
            )
            nodes.append(node)

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
        query_obj = VectorStoreQuery(
            query_embedding=query_dense,
            similarity_top_k=3,
        )
        result = store.query(query_obj)
        for node_with_score in result.nodes:
            text_preview = node_with_score.node.get_content()[:80]
            print(f"  [{node_with_score.score:.4f}] {text_preview}")

        # -- Sparse-only search --
        query_sparse = {42: 2.5, 100: 1.8, 7777: 0.9}
        print("\n=== Sparse-Only Search ===")
        query_obj = VectorStoreQuery(
            query_embedding=None,
            similarity_top_k=3,
        )
        result = store.query(query_obj, sparse_vector=query_sparse)
        for node_with_score in result.nodes:
            text_preview = node_with_score.node.get_content()[:80]
            print(f"  [{node_with_score.score:.4f}] {text_preview}")

        # -- Hybrid search (dense + sparse fused via RRF) --
        print("\n=== Hybrid Search (Dense + Sparse) ===")
        query_obj = VectorStoreQuery(
            query_embedding=query_dense,
            similarity_top_k=5,
        )
        result = store.query(query_obj, sparse_vector=query_sparse)
        for node_with_score in result.nodes:
            text_preview = node_with_score.node.get_content()[:80]
            print(f"  [{node_with_score.score:.4f}] {text_preview}")

        print("\nVelesDB handles dense + sparse + PQ compression in a single engine.")
        print("No separate systems needed for hybrid search or quantization.")
    finally:
        shutil.rmtree(DB_PATH, ignore_errors=True)
