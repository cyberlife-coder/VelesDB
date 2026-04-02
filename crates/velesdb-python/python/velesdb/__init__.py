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
    Database as _RawDatabase,
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

    def search_with_quality(
        self,
        vector: Any,
        quality: str,
        top_k: int = 10,
    ) -> list[dict]:
        """Search with a named quality mode.

        Args:
            vector: Query vector (list or numpy array).
            quality: One of 'fast', 'balanced', 'accurate', 'perfect', 'autotune'.
            top_k: Number of results (default: 10).

        Returns:
            List of dicts with id, score, and payload.
        """
        return self._inner.search_with_quality(vector, quality, top_k)

    def count(self) -> int:
        info = self._inner.info()
        return int(info.get("point_count", 0))

    def get_graph_store(self) -> GraphStore:
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
    "__version__",
]
