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
    validate_named_sparse_vector,
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
from llamaindex_velesdb.scroll_ops import (  # noqa: F401  # pylint: disable=unused-import
    ScrollOpsMixin,
    # `_scroll_one_batch` is re-imported intentionally: tests
    # monkeypatch ``llamaindex_velesdb.vectorstore._scroll_one_batch``
    # to observe pagination, so the symbol must be bound at module
    # scope even though nothing in this file calls it directly.
    _scroll_one_batch,
)

# Re-export for backward compatibility and discoverability.
__all__ = [
    "VelesDBVectorStore",
    "SearchOpsMixin",
    "GraphOpsMixin",
    "ScrollOpsMixin",
    "metadata_filters_to_core_filter",
]

logger = logging.getLogger(__name__)

# Page size for the delete(ref_doc_id) payload scan (VelesQL SELECT
# defaults to LIMIT 10, far too small for chunked documents).
_REF_DOC_SCAN_BATCH = 1000

# Metrics for which VelesDB returns a raw distance (lower = closer). LlamaIndex
# treats `similarities` as higher-is-better, so these are mapped to a bounded,
# monotonically-decreasing similarity; cosine/dot-product are already similarities.
_DISTANCE_METRICS = frozenset({"euclidean", "hamming", "jaccard"})


class VelesDBVectorStore(CollectionAdminMixin, SearchOpsMixin, GraphOpsMixin, ScrollOpsMixin, BasePydanticVectorStore):
    """VelesDB vector store for LlamaIndex.

    A high-performance vector store backed by VelesDB, designed for
    semantic search, RAG applications, and similarity matching.

    Example:
        >>> from llamaindex_velesdb import VelesDBVectorStore
        >>> from llama_index.core import (
        ...     SimpleDirectoryReader, StorageContext, VectorStoreIndex,
        ... )
        >>>
        >>> # Create vector store and wrap it in a StorageContext —
        >>> # from_documents() ignores a bare vector_store= keyword and
        >>> # would silently index into an in-memory store instead.
        >>> vector_store = VelesDBVectorStore(path="./velesdb_data")
        >>> storage_context = StorageContext.from_defaults(
        ...     vector_store=vector_store
        ... )
        >>>
        >>> # Build index from documents (chunks are written to VelesDB)
        >>> documents = SimpleDirectoryReader("data").load_data()
        >>> index = VectorStoreIndex.from_documents(
        ...     documents, storage_context=storage_context
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
            search_quality: Optional default quality preset for all queries:
                ``"fast"``, ``"balanced"``, ``"accurate"``, ``"perfect"``,
                ``"autotune"``, ``"custom:N"``, ``"adaptive:MIN:MAX"``.
                ``None`` uses the built-in search. Per-call override:
                pass ``search_quality=`` to :meth:`query`.
            **kwargs: Additional arguments.

        Raises:
            SecurityError: If any parameter fails validation.
            TypeError: If the removed ``server_url`` parameter is passed.
        """
        if "server_url" in kwargs:
            raise TypeError(
                "server_url has been removed: it was accepted but never "
                "used. VelesDBVectorStore always runs embedded (local "
                "files); to talk to a remote velesdb-server use its REST "
                "API instead."
            )
        # Security: Validate all inputs
        validated_path = validate_path(path)
        validated_collection = validate_collection_name(collection_name)
        validated_metric = validate_metric(metric)
        validated_storage_mode = validate_storage_mode(storage_mode)
        validated_quality: Optional[str] = None
        if search_quality is not None:
            validated_quality = validate_search_quality(search_quality)

        super().__init__(
            path=validated_path,
            storage_mode=validated_storage_mode,
            collection_name=validated_collection,
            metric=validated_metric,
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

    def _score_to_similarity(self, score: float) -> float:
        """Map a raw VelesDB score to a LlamaIndex similarity (higher = closer).

        VelesDB returns a raw *distance* for distance metrics (euclidean, hamming,
        jaccard — lower is closer) and a *similarity* for cosine/dot-product
        (higher is closer). LlamaIndex treats ``similarities`` as higher-is-better
        (``similarity_cutoff``, node postprocessors), so distances are mapped to a
        bounded, monotonically-decreasing similarity; similarity metrics pass
        through unchanged.
        """
        if self.metric.lower() in _DISTANCE_METRICS:
            return 1.0 / (1.0 + max(score, 0.0))
        return score

    def _build_query_result(self, results: list[dict]) -> VectorStoreQueryResult:
        """Build a VectorStoreQueryResult from raw VelesDB result dictionaries."""
        nodes: List[TextNode] = []
        similarities: List[float] = []
        ids: List[str] = []

        for result in results:
            node, score, node_id = self._result_to_parts(result)
            nodes.append(node)
            similarities.append(self._score_to_similarity(score))
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
                if existing_dim == 0 and dimension > 0:
                    raise ValueError(
                        f"Collection '{self.collection_name}' is metadata-only "
                        f"(dimension=0) but requested dimension={dimension}. "
                        f"Use a vector collection."
                    )
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
            **add_kwargs: Additional arguments. ``sparse_vectors`` accepts a
                list aligned with *nodes*; each entry is a flat
                ``dict[int, float]`` or a named ``dict[str, dict[int, float]]``
                mapping (e.g. ``{"bge_m3": {0: 1.5}}``). A named mapping
                creates the named sparse index so it can later be queried with
                ``sparse_index_name="bge_m3"``.

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
                validate_named_sparse_vector(sv)

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
        """Delete all nodes that belong to a reference document.

        Implements the LlamaIndex vector-store protocol: every node whose
        ``ref_doc_id`` payload field matches is removed, so a document that
        was split into N chunks loses all N. The hash of ``ref_doc_id``
        itself is deleted too, covering nodes inserted with the document id
        as their node id.

        Args:
            ref_doc_id: Reference document ID to delete.
            **delete_kwargs: Additional arguments.
        """
        collection = self._collection or self._open_existing_collection()
        if collection is None:
            return

        self._delete_nodes_of_ref_doc(collection, ref_doc_id)
        collection.delete([_stable_hash_id(ref_doc_id)])

    def _open_existing_collection(self) -> Optional[velesdb.Collection]:
        """Bind to the named collection if it already exists on disk."""
        self._collection = self._get_db().get_collection(self.collection_name)
        return self._collection

    def _delete_nodes_of_ref_doc(
        self, collection: velesdb.Collection, ref_doc_id: str
    ) -> None:
        """Delete every node whose payload ``ref_doc_id`` matches."""
        # Parameter binding ($ref) keeps the user value out of the query text
        # entirely. It was a no-op on published wheels <= 1.18.0 (scalar-equality
        # bug); the package now pins velesdb >= 3.8.0 where it works. The only
        # interpolated token is collection_name, regex-validated at construction.
        query_str = (
            f"SELECT * FROM {self.collection_name} "  # nosec B608 — identifier regex-validated
            f"WHERE ref_doc_id = $ref LIMIT {_REF_DOC_SCAN_BATCH}"
        )
        while True:
            rows = collection.query(query_str, {"ref": ref_doc_id})
            if rows:
                collection.delete([row["id"] for row in rows])
            if len(rows) < _REF_DOC_SCAN_BATCH:
                break

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
                validate_named_sparse_vector(sv)

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
        """Retrieve nodes by their LlamaIndex string IDs.

        The string IDs are hashed through ``_stable_hash_id`` before the
        VelesDB lookup, matching the insertion path in ``node_builder``.
        For callers that already hold the hashed integer IDs (e.g. graph
        traversals), use :meth:`get_nodes_by_int_ids` to avoid an extra
        round of hashing.
        """
        if not node_ids:
            return []
        return self.get_nodes_by_int_ids([_stable_hash_id(nid) for nid in node_ids])

    def get_nodes_by_int_ids(self, int_ids: List[int]) -> List[TextNode]:
        """Retrieve nodes by their VelesDB **internal integer point IDs**.

        Use this when callers already hold the int IDs that VelesDB
        stores (e.g. hash-based IDs produced by ``stable_hash_id`` during
        insertion, or IDs returned from a graph traversal). Passing those
        ints back through :meth:`get_nodes` would call ``_stable_hash_id``
        on the string form of an already-hashed int and silently return
        nothing.
        """
        if not int_ids or self._collection is None:
            return []
        points = self._collection.get(int_ids)
        return [self._node_from_result(pt) for pt in points if pt]

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

