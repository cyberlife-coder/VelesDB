"""VelesDB VectorStore implementation for LlamaIndex.

This module provides a LlamaIndex-compatible VectorStore that uses VelesDB
as the underlying vector database for storing and retrieving embeddings.
"""

from __future__ import annotations

import hashlib
import logging
from typing import Any, List, Optional

from llama_index.core.schema import BaseNode, TextNode
from llama_index.core.vector_stores.types import (
    BasePydanticVectorStore,
    VectorStoreQuery,
    VectorStoreQueryResult,
)
from pydantic import ConfigDict, PrivateAttr

import velesdb

from llamaindex_velesdb.security import (
    validate_path,
    validate_k,
    validate_text,
    validate_query,
    validate_metric,
    validate_storage_mode,
    validate_batch_size,
    validate_collection_name,
    validate_weight,
    validate_sparse_vector,
)
from llamaindex_velesdb.filter_ops import metadata_filters_to_core_filter
from llamaindex_velesdb.search_ops import SearchOpsMixin
from llamaindex_velesdb.graph_ops import GraphOpsMixin

# Re-export for backward compatibility and discoverability.
__all__ = [
    "VelesDBVectorStore",
    "SearchOpsMixin",
    "GraphOpsMixin",
    "metadata_filters_to_core_filter",
]

logger = logging.getLogger(__name__)


def _stable_hash_id(value: str) -> int:
    """Generate a stable numeric ID from a string using SHA256.

    Python's hash() is non-deterministic across processes, so we use
    SHA256 for consistent IDs across runs.

    Uses 63 bits from SHA256 for a very low collision probability in
    real-world dataset sizes while keeping a positive integer compatible
    with VelesDB point IDs.

    Args:
        value: String to hash.

    Returns:
        Positive 63-bit integer ID compatible with VelesDB Core.
    """
    hash_bytes = hashlib.sha256(value.encode("utf-8")).digest()
    # Use 8 bytes (64 bits) and clear sign bit to stay in positive i64 range.
    return int.from_bytes(hash_bytes[:8], byteorder="big") & 0x7FFFFFFFFFFFFFFF


class VelesDBVectorStore(SearchOpsMixin, GraphOpsMixin, BasePydanticVectorStore):
    """VelesDB vector store for LlamaIndex.

    A high-performance vector store backed by VelesDB, designed for
    semantic search, RAG applications, and similarity matching.

    Example:
        >>> from llamaindex_velesdb import VelesDBVectorStore
        >>> from llama_index.core import VectorStoreIndex, SimpleDirectoryReader
        >>>
        >>> # Create vector store
        >>> vector_store = VelesDBVectorStore(path="./velesdb_data")
        >>>
        >>> # Build index from documents
        >>> documents = SimpleDirectoryReader("data").load_data()
        >>> index = VectorStoreIndex.from_documents(
        ...     documents, vector_store=vector_store
        ... )
        >>>
        >>> # Query
        >>> query_engine = index.as_query_engine()
        >>> response = query_engine.query("What is VelesDB?")

    Attributes:
        path: Path to the VelesDB database directory.
        collection_name: Name of the collection to use.
        metric: Distance metric (cosine, euclidean, dot).
        storage_mode: Vector storage mode (full, sq8, binary).
    """

    stores_text: bool = True
    flat_metadata: bool = True

    path: str = "./velesdb_data"
    collection_name: str = "llamaindex"
    metric: str = "cosine"
    storage_mode: str = "full"

    _db: Optional[velesdb.Database] = PrivateAttr(default=None)
    _collection: Optional[velesdb.Collection] = PrivateAttr(default=None)
    _dimension: Optional[int] = PrivateAttr(default=None)

    model_config = ConfigDict(arbitrary_types_allowed=True)

    def __init__(
        self,
        path: str = "./velesdb_data",
        collection_name: str = "llamaindex",
        metric: str = "cosine",
        storage_mode: str = "full",
        **kwargs: Any,
    ) -> None:
        """Initialize VelesDB vector store.

        Args:
            path: Path to VelesDB database directory.
            collection_name: Name of the collection.
            metric: Distance metric.
                - "cosine": Cosine similarity (default)
                - "euclidean": Euclidean distance (L2)
                - "dot": Dot product (inner product)
                - "hamming": Hamming distance (for binary vectors)
                - "jaccard": Jaccard similarity (for binary vectors)
            storage_mode: Storage mode ("full", "sq8", "binary").
                - "full": Full f32 precision (default)
                - "sq8": 8-bit scalar quantization (4x memory reduction)
                - "binary": 1-bit binary quantization (32x memory reduction)
            **kwargs: Additional arguments.

        Raises:
            SecurityError: If any parameter fails validation.
        """
        # Security: Validate all inputs
        validated_path = validate_path(path)
        validated_collection = validate_collection_name(collection_name)
        validated_metric = validate_metric(metric)
        validated_storage_mode = validate_storage_mode(storage_mode)

        super().__init__(
            path=validated_path,
            storage_mode=validated_storage_mode,
            collection_name=validated_collection,
            metric=validated_metric,
            **kwargs,
        )

    # ------------------------------------------------------------------
    # Static / class helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _metadata_from_payload(payload: dict) -> dict:
        """Extract metadata from a VelesDB payload."""
        return {k: v for k, v in payload.items() if k not in ("text", "node_id")}

    @classmethod
    def _node_from_result(cls, result: dict) -> TextNode:
        """Convert a VelesDB search result to a TextNode."""
        payload = result.get("payload", {})
        text = payload.get("text", "")
        node_id = payload.get("node_id", str(result.get("id", "")))
        metadata = cls._metadata_from_payload(payload)
        return TextNode(text=text, id_=node_id, metadata=metadata)

    @classmethod
    def _result_to_parts(cls, result: dict) -> tuple[TextNode, float, str]:
        """Convert a VelesDB result into (node, score, node_id)."""
        node = cls._node_from_result(result)
        return node, result.get("score", 0.0), node.node_id

    @classmethod
    def _build_query_result(cls, results: list[dict]) -> VectorStoreQueryResult:
        """Build a VectorStoreQueryResult from raw VelesDB result dictionaries."""
        nodes: List[TextNode] = []
        similarities: List[float] = []
        ids: List[str] = []

        for result in results:
            node, score, node_id = cls._result_to_parts(result)
            nodes.append(node)
            similarities.append(score)
            ids.append(node_id)

        return VectorStoreQueryResult(nodes=nodes, similarities=similarities, ids=ids)

    @classmethod
    def _metadata_filters_to_core_filter(cls, filters: Any) -> Optional[dict]:
        """Convert LlamaIndex MetadataFilters to VelesDB Core filter format."""
        return metadata_filters_to_core_filter(filters)

    # ------------------------------------------------------------------
    # Connection management
    # ------------------------------------------------------------------

    def _get_db(self) -> velesdb.Database:
        """Get or create the database connection."""
        if self._db is None:
            self._db = velesdb.Database(self.path)
        return self._db

    def _get_collection(self, dimension: int) -> velesdb.Collection:
        """Get or create the collection.

        Args:
            dimension: Expected vector dimension.

        Returns:
            The VelesDB collection.

        Raises:
            ValueError: If collection exists with different dimension.
        """
        if self._collection is None or self._dimension != dimension:
            db = self._get_db()
            self._collection = db.get_collection(self.collection_name)
            if self._collection is None:
                self._collection = db.create_collection(
                    self.collection_name,
                    dimension=dimension,
                    metric=self.metric,
                    storage_mode=self.storage_mode,
                )
            else:
                # Validate existing collection dimension matches
                info = self._collection.info()
                existing_dim = info.get("dimension", 0)
                if existing_dim != 0 and existing_dim != dimension:
                    raise ValueError(
                        f"Collection '{self.collection_name}' exists with dimension "
                        f"{existing_dim}, but got vectors of dimension {dimension}. "
                        f"Use a different collection name or matching dimension."
                    )
            self._dimension = dimension
        return self._collection

    @property
    def client(self) -> velesdb.Database:
        """Return the VelesDB client."""
        return self._get_db()

    # ------------------------------------------------------------------
    # Write operations
    # ------------------------------------------------------------------

    def add(
        self,
        nodes: List[BaseNode],
        **add_kwargs: Any,
    ) -> List[str]:
        """Add nodes to the vector store.

        Args:
            nodes: List of nodes with embeddings to add.
            **add_kwargs: Additional arguments.

        Returns:
            List of node IDs that were added.

        Raises:
            SecurityError: If parameters fail validation.
        """
        if not nodes:
            return []

        # Security: Validate batch size
        validate_batch_size(len(nodes))

        # Extract sparse vectors from kwargs (v1.5)
        sparse_vectors = add_kwargs.get("sparse_vectors")
        if sparse_vectors is not None:
            for sv in sparse_vectors:
                validate_sparse_vector(sv)

        # Get dimension from first node's embedding
        first_embedding = nodes[0].get_embedding()
        if first_embedding is None:
            raise ValueError("Nodes must have embeddings")
        dimension = len(first_embedding)

        # When sparse_vectors are provided, all nodes must have embeddings so
        # that sparse_vectors[idx] stays aligned with the built points list.
        if sparse_vectors is not None:
            for i, node in enumerate(nodes):
                if node.get_embedding() is None:
                    raise ValueError(
                        f"Node at index {i} has no embedding. All nodes must have embeddings "
                        f"when sparse_vectors are provided to preserve index alignment."
                    )

        collection = self._get_collection(dimension)
        points = []
        ids = []

        for idx, node in enumerate(nodes):
            embedding = node.get_embedding()
            if embedding is None:
                continue

            node_id = node.node_id
            ids.append(node_id)

            payload = {"text": node.get_content(), "node_id": node_id}

            if hasattr(node, "metadata") and node.metadata:
                for key, value in node.metadata.items():
                    if isinstance(value, (str, int, float, bool)):
                        payload[key] = value

            point = {
                "id": _stable_hash_id(node_id),
                "vector": embedding,
                "payload": payload,
            }

            if sparse_vectors is not None and idx < len(sparse_vectors):
                point["sparse_vector"] = sparse_vectors[idx]

            points.append(point)

        if points:
            collection.upsert(points)

        return ids

    def delete(self, ref_doc_id: str, **delete_kwargs: Any) -> None:
        """Delete nodes by reference document ID.

        Args:
            ref_doc_id: Reference document ID to delete.
            **delete_kwargs: Additional arguments.
        """
        if self._collection is None:
            return

        int_id = _stable_hash_id(ref_doc_id)
        self._collection.delete([int_id])

    def add_bulk(self, nodes: List[BaseNode], **add_kwargs: Any) -> List[str]:
        """Bulk insert optimized for large batches.

        Raises:
            SecurityError: If batch size exceeds limit.
        """
        if not nodes:
            return []

        # Security: Validate batch size
        validate_batch_size(len(nodes))

        first_emb = nodes[0].get_embedding()
        if first_emb is None:
            raise ValueError("Nodes must have embeddings")
        collection = self._get_collection(len(first_emb))

        points, result_ids = [], []
        for node in nodes:
            emb = node.get_embedding()
            if emb is None:
                continue
            nid = node.node_id
            result_ids.append(nid)
            payload = {"text": node.get_content(), "node_id": nid}
            if hasattr(node, "metadata") and node.metadata:
                payload.update({
                    k: v for k, v in node.metadata.items()
                    if isinstance(v, (str, int, float, bool))
                })
            points.append({"id": _stable_hash_id(nid), "vector": emb, "payload": payload})
        if points:
            collection.upsert_bulk(points)
        return result_ids

    def stream_insert(
        self,
        nodes: List[BaseNode],
        **kwargs: Any,
    ) -> int:
        """Insert nodes via streaming channel with backpressure.

        Args:
            nodes: List of nodes with embeddings to insert.
            **kwargs: Additional arguments. Supports 'sparse_vectors' list.

        Returns:
            Number of points inserted.

        Raises:
            SecurityError: If parameters fail validation.
        """
        if not nodes:
            return 0

        validate_batch_size(len(nodes))

        sparse_vectors = kwargs.get("sparse_vectors")
        if sparse_vectors is not None:
            for sv in sparse_vectors:
                validate_sparse_vector(sv)

        first_embedding = nodes[0].get_embedding()
        if first_embedding is None:
            raise ValueError("Nodes must have embeddings")
        dimension = len(first_embedding)

        collection = self._get_collection(dimension)
        points = []

        for idx, node in enumerate(nodes):
            embedding = node.get_embedding()
            if embedding is None:
                raise ValueError(
                    f"Node at index {idx} (id={node.node_id!r}) has no embedding. "
                    f"All nodes passed to stream_insert must have embeddings."
                )

            node_id = node.node_id
            payload = {"text": node.get_content(), "node_id": node_id}

            if hasattr(node, "metadata") and node.metadata:
                for key, value in node.metadata.items():
                    if isinstance(value, (str, int, float, bool)):
                        payload[key] = value

            point = {
                "id": _stable_hash_id(node_id),
                "vector": embedding,
                "payload": payload,
            }

            if sparse_vectors is not None and idx < len(sparse_vectors):
                point["sparse_vector"] = sparse_vectors[idx]

            points.append(point)

        if points:
            collection.stream_insert(points)

        return len(points)

    def add_streaming(
        self,
        nodes: List[BaseNode],
        batch_size: int = 100,
        **add_kwargs: Any,
    ) -> List[str]:
        """Add nodes using streaming insertion for optimal bulk loading.

        Uses VelesDB's stream_insert for better throughput on large datasets.
        Nodes are batched and sent through the streaming ingestion channel
        with built-in backpressure.

        Args:
            nodes: List of nodes with embeddings to add.
            batch_size: Number of points per streaming batch. Defaults to 100.
            **add_kwargs: Additional arguments.

        Returns:
            List of node IDs that were added.

        Raises:
            SecurityError: If parameters fail validation.
            ValueError: If nodes lack embeddings.
        """
        if not nodes:
            return []

        validate_batch_size(len(nodes))

        first_embedding = nodes[0].get_embedding()
        if first_embedding is None:
            raise ValueError("Nodes must have embeddings")
        dimension = len(first_embedding)

        collection = self._get_collection(dimension)
        result_ids: List[str] = []
        batch: list = []

        for node in nodes:
            embedding = node.get_embedding()
            if embedding is None:
                continue

            node_id = node.node_id
            result_ids.append(node_id)

            payload = {"text": node.get_content(), "node_id": node_id}
            if hasattr(node, "metadata") and node.metadata:
                for key, value in node.metadata.items():
                    if isinstance(value, (str, int, float, bool)):
                        payload[key] = value

            batch.append({
                "id": _stable_hash_id(node_id),
                "vector": embedding,
                "payload": payload,
            })

            if len(batch) >= batch_size:
                collection.stream_insert(batch)
                batch = []

        if batch:
            collection.stream_insert(batch)

        return result_ids

    # ------------------------------------------------------------------
    # Read / utility operations
    # ------------------------------------------------------------------

    def get_nodes(self, node_ids: List[str], **kwargs: Any) -> List[TextNode]:
        """Retrieve nodes by their IDs."""
        if not node_ids or self._collection is None:
            return []
        int_ids = [_stable_hash_id(nid) for nid in node_ids]
        points = self._collection.get(int_ids)
        result = []
        for pt in points:
            if pt:
                p = pt.get("payload", {})
                result.append(
                    TextNode(
                        text=p.get("text", ""),
                        id_=p.get("node_id", ""),
                        metadata=self._metadata_from_payload(p),
                    )
                )
        return result

    def get_collection_info(self) -> dict:
        """Get collection configuration information."""
        if self._collection is None:
            return {
                "name": self.collection_name,
                "dimension": 0,
                "metric": self.metric,
                "point_count": 0,
            }
        return self._collection.info()

    def flush(self) -> None:
        """Flush all pending changes to disk."""
        if self._collection is not None:
            self._collection.flush()

    def is_empty(self) -> bool:
        """Check if the collection is empty."""
        return self._collection is None or self._collection.is_empty()

    def create_metadata_collection(self, name: str) -> None:
        """Create a metadata-only collection (no vectors).

        Useful for storing reference data that can be JOINed with
        vector collections (VelesDB Premium feature).

        Args:
            name: Collection name.
        """
        db = self._get_db()
        db.create_metadata_collection(name)

    def is_metadata_only(self) -> bool:
        """Check if the current collection is metadata-only.

        Returns:
            True if metadata-only, False if vector collection.
        """
        if self._collection is None:
            return False
        return self._collection.is_metadata_only()

    def train_pq(self, m: int = 8, k: int = 256, opq: bool = False) -> str:
        """Train Product Quantization on the collection.

        PQ training is a Database-level operation (not Collection-level)
        because TRAIN QUANTIZER requires Database-level VelesQL execution.

        Args:
            m: Number of subspaces. Defaults to 8.
            k: Number of centroids per subspace. Defaults to 256.
            opq: Enable Optimized PQ pre-rotation. Defaults to False.

        Returns:
            Training result message.
        """
        return self._get_db().train_pq(self.collection_name, m=m, k=k, opq=opq)
