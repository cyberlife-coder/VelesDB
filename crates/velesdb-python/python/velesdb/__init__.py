"""Python facade for VelesDB bindings.

This module re-exports the Rust extension API and provides a thin
backward-compatibility layer for legacy call patterns used by tests and
existing SDK consumers.
"""

from __future__ import annotations

import re
import threading
from typing import Any, Iterable

from . import embed

from velesdb.velesdb import (  # type: ignore[attr-defined]
    AgentMemory,
    AutoReindexOptions,
    Collection as _RawCollection,
    CollectionExistsError,
    CollectionNotFoundError,
    Database as _RawDatabase,
    DatabaseLockedError,
    DimensionMismatchError,
    EdgeExistsError,
    FusionStrategy,
    GraphStore as _RawGraphStore,
    HnswOptions,
    LimitsOptions,
    ParsedStatement,
    PyEpisodicMemory,
    GraphCollection as PyGraphCollection,
    GraphSchema as PyGraphSchema,
    PyProceduralMemory,
    PySemanticMemory,
    SearchOptions,
    SearchResult,
    StreamingConfig,
    StreamingIngestConfig,
    TraversalResult,
    VelesConfigOptions,
    VelesDBError,
    VelesQL as _RawVelesQL,
    VelesQLParameterError,
    VelesQLSyntaxError,
    __version__,
)


# Canonical metric names used by the Rust engine (DistanceMetric::canonical_name).
# Maps common aliases to the canonical form so Python-side comparisons are
# consistent regardless of which spelling the user provides.
_METRIC_ALIASES: dict[str, str] = {
    "dotproduct": "dot",
    "dot_product": "dot",
    "inner": "dot",
    "ip": "dot",
    "l2": "euclidean",
    "cos": "cosine",
}


def _normalize_metric(m: str) -> str:
    """Normalize a metric name to its canonical form."""
    key = m.strip().lower()
    return _METRIC_ALIASES.get(key, key)


class GraphStore:
    """Compatibility adapter for GraphStore call shapes."""

    def __init__(self, inner: _RawGraphStore | None = None) -> None:
        self._inner = inner or _RawGraphStore()
        self._legacy_edge_id = 1

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def _next_edge_id(self) -> int:
        edge_id = max(self._legacy_edge_id, self._inner.edge_count() + 1)
        while self.has_edge(edge_id):
            edge_id += 1
        self._legacy_edge_id += 1
        return edge_id

    def has_edge(self, edge_id: int) -> bool:
        return self._inner.has_edge(int(edge_id))

    def add_edge(
        self,
        edge: Any,
        target: int | None = None,
        label: str | None = None,
        weight: Any | None = None,
        *,
        id: int | None = None,
        source: int | None = None,
        properties: dict[str, Any] | None = None,
    ) -> None:
        if isinstance(edge, dict):
            self._inner.add_edge(edge)
            return

        self._inner.add_edge(
            _legacy_edge_to_dict(
                edge,
                target=target,
                label=label,
                weight=weight,
                edge_id=id,
                source=source,
                properties=properties,
                next_id=self._next_edge_id,
            )
        )

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


def _legacy_edge_to_dict(
    edge_or_source: Any,
    *,
    target: int | None,
    label: str | None,
    weight: Any | None,
    edge_id: int | None,
    source: int | None,
    properties: dict[str, Any] | None,
    next_id: Any,
) -> dict[str, Any]:
    """Normalize legacy positional edge calls to the dict contract."""
    props = dict(properties or {})
    if isinstance(weight, dict):
        props.update(weight)
        weight = None
    elif weight is not None:
        props.setdefault("weight", weight)

    # Current explicit-id shape: add_edge(id, source=..., target=..., label=...)
    if source is not None or edge_id is not None:
        eid = int(edge_or_source if edge_id is None else edge_id)
        edge_source = int(source if source is not None else 0)
    else:
        # Legacy no-id shape: add_edge(source, target, label, weight=None)
        eid = int(next_id())
        edge_source = int(edge_or_source)

    if target is None:
        raise ValueError("add_edge() requires a target node id")

    edge_dict: dict[str, Any] = {
        "id": eid,
        "source": edge_source,
        "target": int(target),
        "label": label or "RELATED_TO",
    }
    if props:
        edge_dict["properties"] = props
    return edge_dict


class GraphCollection:
    """Compatibility adapter around the Rust GraphCollection binding."""

    def __init__(self, inner: PyGraphCollection) -> None:
        self._inner = inner
        self._legacy_edge_id = 1

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def __contains__(self, node_id: int) -> bool:
        return self._inner.__contains__(int(node_id))

    def __enter__(self) -> "GraphCollection":
        return self

    def __exit__(self, _exc_type: Any, _exc_value: Any, _traceback: Any) -> bool:
        self._inner.close()
        return False

    def _next_edge_id(self) -> int:
        edge_id = max(self._legacy_edge_id, self._inner.edge_count() + 1)
        while self.has_edge(edge_id):
            edge_id += 1
        self._legacy_edge_id += 1
        return edge_id

    def has_edge(self, edge_id: int) -> bool:
        return self._inner.has_edge(int(edge_id))

    def add_edge(
        self,
        edge: Any,
        target: int | None = None,
        label: str | None = None,
        weight: Any | None = None,
        *,
        id: int | None = None,
        source: int | None = None,
        properties: dict[str, Any] | None = None,
    ) -> None:
        if isinstance(edge, dict):
            self._inner.add_edge(edge)
            return

        self._inner.add_edge(
            _legacy_edge_to_dict(
                edge,
                target=target,
                label=label,
                weight=weight,
                edge_id=id,
                source=source,
                properties=properties,
                next_id=self._next_edge_id,
            )
        )

    def add_node(
        self,
        node_id: int,
        payload: dict[str, Any] | None = None,
        vector: Iterable[float] | None = None,
    ) -> None:
        self._inner.upsert_node(int(node_id), dict(payload or {}), vector)

    def bfs(self, start_id: int, max_depth: int = 3, limit: int = 100) -> list[dict[str, Any]]:
        return self._inner.traverse_bfs(start_id, max_depth=max_depth, limit=limit)

    def dfs(self, start_id: int, max_depth: int = 3, limit: int = 100) -> list[dict[str, Any]]:
        return self._inner.traverse_dfs(start_id, max_depth=max_depth, limit=limit)

    def close(self) -> None:
        self._inner.close()


class Collection:
    """Compatibility adapter around the Rust Collection binding."""

    def __init__(self, inner: _RawCollection) -> None:
        self._inner = inner
        self._graph_store: GraphStore | None = None

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def __len__(self) -> int:
        return self._inner.__len__()

    # Pythonic protocols (#426) — must live on the wrapper class because
    # CPython resolves dunder methods via slot lookup on type(obj),
    # bypassing __getattr__ delegation.
    def __contains__(self, point_id: int) -> bool:
        return self._inner.__contains__(int(point_id))

    def __enter__(self) -> "Collection":
        return self

    def __exit__(self, _exc_type: Any, _exc_value: Any, _traceback: Any) -> bool:
        self._inner.close()
        return False

    def close(self) -> None:
        """Graceful shutdown: full durability flush including ``vectors.idx``.

        Idempotent — safe to call multiple times.
        """
        self._inner.close()

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

    def search_request(self, opts: SearchOptions) -> list[dict[str, Any]]:
        """Search using a :py:class:`SearchOptions` builder (v1.15+).

        Preferred over :py:meth:`search` for new code.

        Args:
            opts: Populated :py:class:`SearchOptions` instance.

        Returns:
            List of dicts with ``id``, ``score``, and ``payload`` keys.
        """
        return self._inner.search_request(opts)

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


def _safe_len(vec: Any) -> int | None:
    """Return the length of ``vec`` if measurable, else None."""
    try:
        return len(list(vec))
    except TypeError:
        return None


def _dimension_from_points(points_or_id: Any) -> int | None:
    """Detect the vector dimension from the first point of an upsert list."""
    if not (isinstance(points_or_id, list) and points_or_id):
        return None
    first = points_or_id[0]
    if not isinstance(first, dict):
        return None
    vec = first.get("vector")
    return None if vec is None else _safe_len(vec)


def _extract_dimension(points_or_id: Any, vector: Iterable[float] | None) -> int | None:
    """Return the vector dimension detectable from upsert arguments, or None."""
    if vector is not None:
        return _safe_len(vector)
    return _dimension_from_points(points_or_id)


class _PendingCollection:
    """Deferred vector collection whose dimension is auto-detected from the first upsert.

    Returned by :meth:`Database.create_collection` when ``dimension=None``.
    The underlying Rust collection is created lazily on the first call that
    supplies a vector, ensuring the caller never needs to know the dimension
    upfront.  All subsequent calls are transparently forwarded to the real
    :class:`Collection`.

    Thread-safe: a ``threading.Lock`` guards the one-time materialisation.
    """

    def __init__(
        self,
        db: "Database",
        name: str,
        metric: str,
        storage_mode: str,
        hnsw: Any,
        auto_reindex: Any,
    ) -> None:
        self._db = db
        self._name = name
        self._metric = metric
        self._storage_mode = storage_mode
        self._hnsw = hnsw
        self._auto_reindex = auto_reindex
        self._collection: Collection | None = None
        self._lock = threading.Lock()

    def _materialize(self, dimension: int) -> "Collection":
        with self._lock:
            if self._collection is None:
                self._collection = self._db._create_collection_with_dim(
                    self._name,
                    dimension,
                    self._metric,
                    self._storage_mode,
                    self._hnsw,
                    self._auto_reindex,
                )
        return self._collection  # type: ignore[return-value]

    def upsert(
        self,
        points_or_id: Any,
        vector: Iterable[float] | None = None,
        payload: dict[str, Any] | None = None,
    ) -> int:
        # Materialise *before* consuming the iterator so the vector is intact.
        dim = _extract_dimension(points_or_id, vector)
        if dim is None:
            raise ValueError(
                f"Cannot auto-detect dimension for collection '{self._name}': "
                "the first upsert must include at least one point with a 'vector' key, "
                "or pass the 'vector' argument directly."
            )
        return self._materialize(dim).upsert(points_or_id, vector, payload)

    def upsert_bulk_numpy(
        self,
        vectors: Any,
        ids: Any,
        payloads: Any = None,
    ) -> int:
        shape = getattr(vectors, "shape", None)
        dim = shape[1] if shape and len(shape) >= 2 else None
        if dim is None and vectors:
            dim = len(vectors[0])
        if not dim:
            raise ValueError(
                f"Cannot auto-detect dimension for collection '{self._name}' "
                "from upsert_bulk_numpy: vectors array is empty or has no shape."
            )
        return self._materialize(dim).upsert_bulk_numpy(vectors, ids, payloads)

    def upsert_from_dataframe(self, df: Any, **kwargs: Any) -> int:
        vector_col = kwargs.get("vector_column", "vector")
        try:
            first_vec = df[vector_col].iloc[0]  # pandas
        except AttributeError:
            first_vec = df[vector_col][0]  # polars
        dim = len(first_vec)
        return self._materialize(dim).upsert_from_dataframe(df, **kwargs)

    def __getattr__(self, name: str) -> Any:
        if self._collection is not None:
            return getattr(self._collection, name)
        raise AttributeError(
            f"Collection '{self._name}' has no dimension yet — "
            f"call upsert() with a vector to auto-detect dimension before calling '{name}'."
        )

    def _require_materialized(self, op: str) -> "Collection":
        """Return the materialised collection or raise a guiding error.

        Dunder methods (``len()``, ``in``, ``with``) are resolved on the type,
        not via :meth:`__getattr__`, so they must be forwarded explicitly once
        the underlying collection exists. (Mirrors :class:`Collection`, which is
        not iterable — use ``scroll()`` to stream points.)
        """
        if self._collection is None:
            raise RuntimeError(
                f"Collection '{self._name}' has no dimension yet — call upsert() "
                f"with a vector to auto-detect dimension before using {op}."
            )
        return self._collection

    def __len__(self) -> int:
        return len(self._require_materialized("len()"))

    def __contains__(self, point_id: Any) -> bool:
        return point_id in self._require_materialized("'in'")

    def __enter__(self) -> "Collection":
        return self._require_materialized("'with'").__enter__()

    def __exit__(self, exc_type: Any, exc_value: Any, traceback: Any) -> bool:
        return self._require_materialized("'with'").__exit__(exc_type, exc_value, traceback)

    def __repr__(self) -> str:
        return f"Collection(name={self._name!r}, dimension=<pending>)"


class Database:
    """Compatibility adapter around the Rust Database binding."""

    def __init__(
        self,
        path: str,
        config: VelesConfigOptions | None = None,
    ) -> None:
        self._inner = _RawDatabase(path, config)

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def create_collection(
        self,
        name: str,
        dimension: int | None = None,
        metric: str = "cosine",
        storage_mode: str = "full",
        hnsw: HnswOptions | None = None,
        auto_reindex: AutoReindexOptions | None = None,
    ) -> "Collection | _PendingCollection":
        """Create a new vector collection.

        Args:
            name: Collection name.
            dimension: Vector dimension (e.g. 768 for BERT embeddings).
                Pass ``None`` (default) to auto-detect from the first
                ``upsert()`` call — no need to know the dimension upfront.
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

            hnsw: Optional :class:`HnswOptions` dataclass with typed HNSW
                parameters. Replaces the legacy v1.12 flat kwargs
                (``m=``, ``ef_construction=``, ``expected_vectors=``) —
                see the v1.13 CHANGELOG for the migration guide.
            auto_reindex: Optional :class:`AutoReindexOptions` dataclass.
                When provided, an ``AutoReindexManager`` is constructed and
                attached to the freshly-created collection as a runtime-only
                hook (not persisted — re-attach after every ``Database(path)``).

        Returns:
            :class:`Collection` when ``dimension`` is known, or a
            :class:`_PendingCollection` that auto-detects dimension on the
            first ``upsert()`` call when ``dimension=None``.
        """
        if dimension is None:
            return _PendingCollection(
                self, name, metric, storage_mode, hnsw, auto_reindex
            )
        return self._create_collection_with_dim(
            name, dimension, metric, storage_mode, hnsw, auto_reindex
        )

    def _create_collection_with_dim(
        self,
        name: str,
        dimension: int,
        metric: str,
        storage_mode: str,
        hnsw: HnswOptions | None,
        auto_reindex: AutoReindexOptions | None,
    ) -> "Collection":
        col = self._inner.create_collection(
            name, dimension, metric, storage_mode, hnsw, auto_reindex
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
        dimension: int | None = None,
        metric: str = "cosine",
        storage_mode: str = "full",
        hnsw: HnswOptions | None = None,
        auto_reindex: AutoReindexOptions | None = None,
    ) -> "Collection | _PendingCollection":
        """Return an existing collection or create it if missing.

        When the collection already exists, its ``dimension`` and ``metric``
        are validated against the requested parameters (when ``dimension`` is
        provided). A ``ValueError`` is raised on mismatch. Other parameters
        (``storage_mode``, ``hnsw``, ``auto_reindex``) are ignored for the
        lookup path.

        Pass ``dimension=None`` to auto-detect from the first ``upsert()``
        call when creating a new collection, or to skip dimension validation
        when opening an existing one.

        Args and accepted storage modes are identical to :meth:`create_collection`.

        Returns:
            Existing :class:`Collection` if found, otherwise a freshly created
            one (or a :class:`_PendingCollection` when ``dimension=None`` and
            the collection does not yet exist).

        Raises:
            ValueError: If the existing collection has a different dimension
                or metric (only when ``dimension`` is not ``None``).
        """
        existing = self.get_collection(name)
        if existing is not None:
            if dimension is not None:
                existing_dim = existing._inner.dimension
                existing_metric = existing._inner.metric
                if existing_dim != dimension:
                    raise ValueError(
                        f"Collection '{name}' exists with dimension {existing_dim}, "
                        f"but requested dimension {dimension}. "
                        f"Use a different name or matching parameters."
                    )
                if _normalize_metric(existing_metric) != _normalize_metric(metric):
                    raise ValueError(
                        f"Collection '{name}' exists with metric '{existing_metric}', "
                        f"but requested metric '{metric}'. "
                        f"Use a different name or matching parameters."
                    )
            return existing
        return self.create_collection(
            name,
            dimension=dimension,
            metric=metric,
            storage_mode=storage_mode,
            hnsw=hnsw,
            auto_reindex=auto_reindex,
        )

    def create_metadata_collection(self, name: str) -> "Collection":
        col = self._inner.create_metadata_collection(name)
        return Collection(col)

    def create_graph_collection(
        self,
        name: str,
        dimension: int | None = None,
        metric: str = "cosine",
        schema: PyGraphSchema | None = None,
    ) -> GraphCollection:
        graph = self._inner.create_graph_collection(
            name,
            dimension=dimension,
            metric=metric,
            schema=schema,
        )
        return GraphCollection(graph)

    def get_graph_collection(self, name: str) -> "GraphCollection | None":
        graph = self._inner.get_graph_collection(name)
        if graph is None:
            return None
        return GraphCollection(graph)

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
            r"USING\s+FUSION\s+([a-z_][a-z0-9_]*)\b",
            r"USING FUSION (strategy='\1')",
            normalized,
            flags=re.IGNORECASE,
        )
        # The FROM/JOIN look-aheads enumerate VelesQL clause keywords
        # (JOIN/WHERE/GROUP/ORDER/LIMIT/OFFSET) to mark where an alias ends. That
        # alternation is grammar-driven and cannot be simplified without changing
        # what the patterns match (the common `\s+` prefix is already factored
        # out). The resulting regex-complexity rule (python:S5843) is suppressed
        # for this file in sonar-project.properties with that justification.
        normalized = re.sub(
            r"\bFROM\s+([a-z_][a-z0-9_]*)\s+([a-z_][a-z0-9_]*)\b(?=\s+(?:JOIN|WHERE|GROUP|ORDER|LIMIT|OFFSET)|$)",
            r"FROM \1 AS \2",
            normalized,
            flags=re.IGNORECASE,
        )
        normalized = re.sub(
            r"\bJOIN\s+([a-z_][a-z0-9_]*)\s+([a-z_][a-z0-9_]*)\b(?=\s+(?:ON|WHERE|GROUP|ORDER|LIMIT|OFFSET)|$)",
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
    "SearchOptions",
    "SearchResult",
    "FusionStrategy",
    "GraphStore",
    "GraphCollection",
    "StreamingConfig",
    "StreamingIngestConfig",
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
    # Typed options dataclasses (Wave 3 Commit 10). `HnswOptions` and
    # `AutoReindexOptions` are passed to `Database.create_collection`;
    # `LimitsOptions` wraps tenant-wide guard-rails; `VelesConfigOptions`
    # is the database-level wrapper passed to `Database(path, config=...)`.
    "HnswOptions",
    "LimitsOptions",
    "AutoReindexOptions",
    "VelesConfigOptions",
    "embed",
    "__version__",
]
