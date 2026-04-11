"""Python facade for VelesDB bindings.

This module re-exports the Rust extension API and provides a thin
backward-compatibility layer for legacy call patterns used by tests and
existing SDK consumers.
"""

from __future__ import annotations

import re
from typing import Any, Iterable

from velesdb.velesdb import (  # type: ignore[attr-defined]
    AgentMemory,
    Collection as _RawCollection,
    CollectionExistsError,
    CollectionNotFoundError,
    Database as _RawDatabase,
    DatabaseLockedError,
    DimensionMismatchError,
    EdgeExistsError,
    FusionStrategy,
    GraphStore as _RawGraphStore,
    ParsedStatement,
    PyEpisodicMemory,
    GraphCollection as PyGraphCollection,
    GraphSchema as PyGraphSchema,
    PyProceduralMemory,
    PySemanticMemory,
    SearchResult,
    StreamingConfig,
    TraversalResult,
    VelesDBError,
    VelesQL as _RawVelesQL,
    VelesQLParameterError,
    VelesQLSyntaxError,
    __version__,
)


class GraphStore:
    """Compatibility adapter for GraphStore call shapes."""

    def __init__(self, inner: _RawGraphStore | None = None) -> None:
        self._inner = inner or _RawGraphStore()

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def add_edge(
        self,
        edge: Any,
        *,
        source: int | None = None,
        target: int | None = None,
        label: str | None = None,
        properties: dict[str, Any] | None = None,
    ) -> None:
        if isinstance(edge, dict):
            self._inner.add_edge(edge)
            return

        edge_dict: dict[str, Any] = {
            "id": int(edge),
            "source": int(source) if source is not None else 0,
            "target": int(target) if target is not None else 0,
            "label": label or "",
        }
        if properties is not None:
            edge_dict["properties"] = properties
        self._inner.add_edge(edge_dict)

    def traverse_bfs(
        self,
        *,
        source: int,
        max_depth: int = 3,
        limit: int = 10_000,
        relationship_types: list[str] | None = None,
    ) -> list[TraversalResult]:
        cfg = StreamingConfig(
            max_depth=max_depth,
            max_visited=limit,
            relationship_types=relationship_types,
        )
        return self._inner.traverse_bfs_streaming(source, cfg)

    def traverse_dfs(
        self,
        *,
        source: int,
        max_depth: int = 3,
        limit: int = 10_000,
        relationship_types: list[str] | None = None,
    ) -> list[TraversalResult]:
        cfg = StreamingConfig(
            max_depth=max_depth,
            max_visited=limit,
            relationship_types=relationship_types,
        )
        return self._inner.traverse_dfs(source, cfg)


class Collection:
    """Compatibility adapter around the Rust Collection binding."""

    def __init__(self, inner: _RawCollection) -> None:
        self._inner = inner
        self._graph_store: GraphStore | None = None

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def __len__(self) -> int:
        return self._inner.__len__()

    def upsert(
        self,
        points_or_id: Any,
        vector: Iterable[float] | None = None,
        payload: dict[str, Any] | None = None,
    ) -> int:
        if vector is None:
            return self._inner.upsert(points_or_id)

        point = {"id": int(points_or_id), "vector": list(vector)}
        if payload is not None:
            point["payload"] = payload
        return self._inner.upsert([point])

    def search(
        self,
        vector: Iterable[float] | None = None,
        top_k: int = 10,
        filter: dict[str, Any] | None = None,
        sparse_vector: Any | None = None,
        sparse_index_name: str | None = None,
    ) -> list[dict[str, Any]]:
        kwargs: dict[str, Any] = {"top_k": top_k}
        if vector is not None:
            kwargs["vector"] = list(vector)
        if filter is not None:
            kwargs["filter"] = filter
        if sparse_vector is not None:
            kwargs["sparse_vector"] = sparse_vector
        if sparse_index_name is not None:
            kwargs["sparse_index_name"] = sparse_index_name
        return self._inner.search(**kwargs)

    def batch_search(
        self,
        queries: list[Any],
        top_k: int = 10,
    ) -> list[list[dict[str, Any]]]:
        if queries and isinstance(queries[0], dict):
            # Pass dicts through to Rust, injecting the default top_k only
            # when the caller omitted both "top_k" and "topK".
            searches = []
            for q in queries:
                entry = dict(q)  # shallow copy to avoid mutating caller's dict
                if "top_k" not in entry and "topK" not in entry:
                    entry["top_k"] = top_k
                searches.append(entry)
        else:
            searches = [{"vector": list(v), "top_k": int(top_k)} for v in queries]
        return self._inner.batch_search(searches)

    def multi_query_search(
        self,
        vectors: list[list[float]],
        top_k: int = 10,
        fusion: Any = None,
        filter: dict[str, Any] | None = None,
    ) -> list[dict[str, Any]]:
        """Multi-query search with result fusion.

        Args:
            vectors: List of query vectors.
            top_k: Number of results per query (default: 10).
            fusion: Optional FusionStrategy instance.
            filter: Optional metadata filter dict.
        """
        return self._inner.multi_query_search(
            vectors, top_k=top_k, fusion=fusion, filter=filter,
        )

    def stream_insert(self, points: list[dict[str, Any]]) -> int:
        """Streaming insert for real-time ingestion."""
        return self._inner.stream_insert(points)

    def search_batch_parallel(
        self,
        vectors: list[list[float]],
        top_k: int = 10,
    ) -> list[list[dict[str, Any]]]:
        """Parallel batch search using rayon."""
        return self._inner.search_batch_parallel(vectors, top_k=top_k)

    def search_with_quality(
        self,
        vector: Any,
        quality: str,
        top_k: int = 10,
    ) -> list[dict]:
        """Search with a named quality mode.

        Args:
            vector: Query vector (list or numpy array).
            quality: One of 'fast', 'balanced', 'accurate', 'perfect', 'autotune',
                     'custom:<ef>' (e.g. 'custom:256'), or 'adaptive:<min>:<max>'
                     (e.g. 'adaptive:32:512').
            top_k: Number of results (default: 10).

        Returns:
            List of dicts with id, score, and payload.
        """
        return self._inner.search_with_quality(vector, quality, top_k)

    def count(self) -> int:
        """Return the number of points in the collection."""
        return self._inner.count()

    def get_graph_store(self) -> GraphStore:
        """Return a standalone in-memory graph store.

        Warning:
            This graph store is **independent** of this collection's data.
            Edges and nodes added here are NOT persisted to the collection.
            For persistent graph operations, use
            ``Database.get_graph_collection()`` instead.
        """
        import warnings

        warnings.warn(
            "Collection.get_graph_store() returns a standalone in-memory "
            "graph not connected to this collection. Use "
            "Database.get_graph_collection() for persistent graph operations.",
            DeprecationWarning,
            stacklevel=2,
        )
        if self._graph_store is None:
            self._graph_store = GraphStore()
        return self._graph_store


class Database:
    """Compatibility adapter around the Rust Database binding."""

    def __init__(self, path: str) -> None:
        self._inner = _RawDatabase(path)

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine",
        storage_mode: str = "full",
        m: int | None = None,
        ef_construction: int | None = None,
        expected_vectors: int | None = None,
    ) -> "Collection":
        """Create a new vector collection.

        Args:
            name: Collection name.
            dimension: Vector dimension (e.g. 768 for BERT embeddings).
            metric: Distance metric (default ``"cosine"``). One of ``"cosine"``,
                ``"euclidean"``, ``"dot"``, ``"hamming"``, ``"jaccard"``.
            storage_mode: Storage mode (default ``"full"``). Accepted values
                (case-insensitive; aliases in parentheses):

                - ``"full"`` (``"f32"``): Full f32 precision — best recall.
                - ``"sq8"`` (``"int8"``): 8-bit scalar quantization — 4x compression.
                - ``"binary"`` (``"bit"``): 1-bit binary quantization — 32x compression.
                - ``"pq"`` (``"product_quantization"``): Product Quantization — 8x-16x
                  compression via trained codebooks (requires a training step).
                - ``"rabitq"``: RaBitQ — 1-bit with rotation + scalar correction,
                  32x compression with ~1-2% recall loss.

            m: HNSW ``max_connections`` parameter (overrides adaptive default).
            ef_construction: HNSW ``ef_construction`` parameter (overrides adaptive default).
            expected_vectors: Expected dataset size used to auto-tune ``m`` and
                ``ef_construction`` when they are not explicitly set.

        Returns:
            Collection instance wrapping the underlying Rust collection.
        """
        col = self._inner.create_collection(
            name, dimension, metric, storage_mode, m, ef_construction, expected_vectors
        )
        return Collection(col)

    def get_collection(self, name: str) -> "Collection | None":
        col = self._inner.get_collection(name)
        if col is None:
            return None
        return Collection(col)

    def get_or_create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine",
        storage_mode: str = "full",
        m: int | None = None,
        ef_construction: int | None = None,
        expected_vectors: int | None = None,
    ) -> "Collection":
        """Return an existing collection or create it if missing.

        When the collection already exists, it is returned as-is — ``dimension``,
        ``metric``, and ``storage_mode`` are ignored for the lookup path and no
        compatibility check is performed against the stored configuration.

        Args and accepted storage modes are identical to :meth:`create_collection`.

        Returns:
            Existing :class:`Collection` if found, otherwise a freshly created one.
        """
        existing = self.get_collection(name)
        if existing is not None:
            return existing
        return self.create_collection(
            name, dimension=dimension, metric=metric, storage_mode=storage_mode,
            m=m, ef_construction=ef_construction, expected_vectors=expected_vectors,
        )

    def create_metadata_collection(self, name: str) -> "Collection":
        col = self._inner.create_metadata_collection(name)
        return Collection(col)

    def execute_query(
        self,
        sql: str,
        params: dict | None = None,
    ) -> list[dict]:
        """Execute a VelesQL query string (SELECT, DDL, DML).

        Supports all VelesQL statements including:

        - SELECT ... FROM ... WHERE ...
        - CREATE [GRAPH|METADATA] COLLECTION ...
        - DROP COLLECTION [IF EXISTS] ...
        - INSERT EDGE INTO ...
        - DELETE FROM ... WHERE ...
        - DELETE EDGE ... FROM ...

        Args:
            sql: VelesQL query string.
            params: Optional parameter bindings (e.g., {"$v": [0.1, 0.2]}).

        Returns:
            List of result dicts for SELECT queries, empty list for DDL/DML.

        Raises:
            ValueError: If parsing fails.
            RuntimeError: If execution fails.
        """
        if params is None:
            params = {}
        return self._inner.execute_query(sql, params)


class VelesQL:
    """Compatibility wrapper for VelesQL parser API."""

    def __init__(self) -> None:
        # Legacy code instantiates VelesQL(), while current API is static-only.
        pass

    @staticmethod
    def _normalize_legacy_query(query: str) -> str:
        normalized = query
        normalized = re.sub(
            r"USING\s+FUSION\s+([a-zA-Z_][a-zA-Z0-9_]*)\b",
            r"USING FUSION (strategy='\1')",
            normalized,
            flags=re.IGNORECASE,
        )
        normalized = re.sub(
            r"\bFROM\s+([A-Za-z_][A-Za-z0-9_]*)\s+([A-Za-z_][A-Za-z0-9_]*)\b(?=\s+JOIN|\s+WHERE|\s+GROUP|\s+ORDER|\s+LIMIT|\s+OFFSET|$)",
            r"FROM \1 AS \2",
            normalized,
            flags=re.IGNORECASE,
        )
        normalized = re.sub(
            r"\bJOIN\s+([A-Za-z_][A-Za-z0-9_]*)\s+([A-Za-z_][A-Za-z0-9_]*)\b(?=\s+ON|\s+WHERE|\s+GROUP|\s+ORDER|\s+LIMIT|\s+OFFSET|$)",
            r"JOIN \1 AS \2",
            normalized,
            flags=re.IGNORECASE,
        )
        return normalized

    @staticmethod
    def parse(query: str) -> ParsedStatement:
        try:
            return _RawVelesQL.parse(query)
        except (VelesQLSyntaxError, VelesQLParameterError):
            normalized = VelesQL._normalize_legacy_query(query)
            if normalized == query:
                raise
            return _RawVelesQL.parse(normalized)

    @staticmethod
    def is_valid(query: str) -> bool:
        return _RawVelesQL.is_valid(VelesQL._normalize_legacy_query(query))


__all__ = [
    "Database",
    "Collection",
    "SearchResult",
    "FusionStrategy",
    "GraphStore",
    "StreamingConfig",
    "TraversalResult",
    "VelesQL",
    "ParsedStatement",
    "VelesQLSyntaxError",
    "VelesQLParameterError",
    "AgentMemory",
    "PySemanticMemory",
    "PyEpisodicMemory",
    "PyProceduralMemory",
    "PyGraphCollection",
    "PyGraphSchema",
    # Typed VelesDB exception hierarchy (all inherit from `VelesDBError`).
    # Import them via `import velesdb` and catch the specific subclass
    # for actionable error handling; use `VelesDBError` as a catch-all.
    "VelesDBError",
    "CollectionNotFoundError",
    "CollectionExistsError",
    "DimensionMismatchError",
    "EdgeExistsError",
    "DatabaseLockedError",
    "__version__",
]
