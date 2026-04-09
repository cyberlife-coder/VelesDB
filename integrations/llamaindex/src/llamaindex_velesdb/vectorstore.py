"""VelesDB VectorStore implementation for LlamaIndex.

This module provides a LlamaIndex-compatible VectorStore that uses VelesDB
as the underlying vector database for storing and retrieving embeddings.

Node construction and streaming helpers live in a dedicated module:
- :mod:`llamaindex_velesdb.node_builder` — build_points_with_ids, flush_in_batches, etc.
"""

from __future__ import annotations

import logging
from typing import Any, List, Optional

from llama_index.core.schema import BaseNode, TextNode
from llama_index.core.vector_stores.types import (
    BasePydanticVectorStore,
    VectorStoreQueryResult,
)
from pydantic import ConfigDict, PrivateAttr

import velesdb

from llamaindex_velesdb.security import (
    validate_path,
    validate_metric,
    validate_search_quality,
    validate_storage_mode,
    validate_batch_size,
    validate_collection_name,
    validate_sparse_vector,
    validate_url,
)
from velesdb_common.collection_admin import CollectionAdminMixin
from velesdb_common.ids import stable_hash_id as _stable_hash_id
from llamaindex_velesdb.filter_ops import metadata_filters_to_core_filter
from llamaindex_velesdb.node_builder import (
    validate_all_embeddings as _validate_all_embeddings,
    build_points_with_ids as _build_points_with_ids,
    flush_in_batches as _flush_in_batches,
    build_stream_points as _build_stream_points,
)
from llamaindex_velesdb.search_ops import SearchOpsMixin
from llamaindex_velesdb.graph_ops import GraphOpsMixin
from llamaindex_velesdb.scroll_ops import ScrollOpsMixin, _scroll_one_batch  # noqa: F401

# Re-export for backward compatibility and discoverability.
__all__ = [
    "VelesDBVectorStore",
    "SearchOpsMixin",
    "GraphOpsMixin",
    "ScrollOpsMixin",
    "metadata_filters_to_core_filter",
]

logger = logging.getLogger(__name__)


class VelesDBVectorStore(CollectionAdminMixin, SearchOpsMixin, GraphOpsMixin, ScrollOpsMixin, BasePydanticVectorStore):
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
        storage_mode: Vector storage mode (full, sq8, binary, pq, rabitq) or alias (f32, int8, bit, product_quantization, product-quantization).
    """

    stores_text: bool = True
    flat_metadata: bool = True

    path: str = "./velesdb_data"
    collection_name: str = "llamaindex"
    metric: str = "cosine"
    storage_mode: str = "full"
    server_url: Optional[str] = None
    search_quality: Optional[str] = None

    _db: Optional[velesdb.Database] = PrivateAttr(default=None)
    _collection: Optional[velesdb.Collection] = PrivateAttr(default=None)
    _dimension: Optional[int] = PrivateAttr(default=None)
    _search_quality: Optional[str] = PrivateAttr(default=None)

    model_config = ConfigDict(arbitrary_types_allowed=True)

    def __init__(
        self,
        path: str = "./velesdb_data",
        collection_name: str = "llamaindex",
        metric: str = "cosine",
        storage_mode: str = "full",
        server_url: Optional[str] = None,
        search_quality: Optional[str] = None,
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
            storage_mode: Storage mode — canonical name or alias (case-insensitive).
                Canonical names:

                - "full": Full f32 precision (default). Alias: "f32".
                - "sq8": 8-bit scalar quantization (4x memory reduction). Alias: "int8".
                - "binary": 1-bit binary quantization (32x memory reduction). Alias: "bit".
                - "pq": Product quantization (8-32x compression, best for large-scale
                  datasets). Aliases: "product_quantization", "product-quantization".
                - "rabitq": RaBitQ with scalar correction (32x compression, good recall).

                Examples: ``storage_mode="int8"`` is equivalent to ``storage_mode="sq8"``.
            server_url: Optional URL of a VelesDB server for server mode. When
                provided, must be a valid http:// or https:// URL.
            search_quality: Optional default quality preset for all queries:
                ``"fast"``, ``"balanced"``, ``"accurate"``, ``"perfect"``,
                ``"autotune"``, ``"custom:N"``, ``"adaptive:MIN:MAX"``.
                ``None`` uses the built-in search. Per-call override:
                pass ``search_quality=`` to :meth:`query`.
            **kwargs: Additional arguments.

        Raises:
            SecurityError: If any parameter fails validation.
        """
        # Security: Validate all inputs
        validated_path = validate_path(path)
        validated_collection = validate_collection_name(collection_name)
        validated_metric = validate_metric(metric)
        validated_storage_mode = validate_storage_mode(storage_mode)
        if server_url is not None:
            validate_url(server_url)
        validated_quality: Optional[str] = None
        if search_quality is not None:
            validated_quality = validate_search_quality(search_quality)

        super().__init__(
            path=validated_path,
            storage_mode=validated_storage_mode,
            collection_name=validated_collection,
            metric=validated_metric,
            server_url=server_url,
            search_quality=validated_quality,
            **kwargs,
        )
        self._search_quality = validated_quality

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

        validate_batch_size(len(nodes))

        sparse_vectors = add_kwargs.get("sparse_vectors")
        if sparse_vectors is not None:
            for sv in sparse_vectors:
                validate_sparse_vector(sv)

        first_embedding = nodes[0].get_embedding()
        if first_embedding is None:
            raise ValueError("Nodes must have embeddings")
        dimension = len(first_embedding)

        if sparse_vectors is not None:
            _validate_all_embeddings(nodes)

        collection = self._get_collection(dimension)
        points, ids = _build_points_with_ids(nodes, sparse_vectors)

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

        validate_batch_size(len(nodes))

        first_emb = nodes[0].get_embedding()
        if first_emb is None:
            raise ValueError("Nodes must have embeddings")
        collection = self._get_collection(len(first_emb))

        points, result_ids = _build_points_with_ids(nodes)
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

        collection = self._get_collection(len(first_embedding))
        points = _build_stream_points(nodes, sparse_vectors)

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

        collection = self._get_collection(len(first_embedding))
        points, result_ids = _build_points_with_ids(nodes)

        _flush_in_batches(collection, points, batch_size)

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

