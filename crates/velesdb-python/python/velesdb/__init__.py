"""Python facade for VelesDB bindings.

This module re-exports the Rust extension API and provides a thin
backward-compatibility layer for legacy call patterns used by tests and
existing SDK consumers.
"""

from __future__ import annotations

from typing import Any, Dict, Iterable, List, Optional

from velesdb.velesdb import (  # type: ignore[attr-defined]
    AgentMemory,
    Collection as _RawCollection,
    Database as _RawDatabase,
    FusionStrategy,
    GraphStore as _RawGraphStore,
    ParsedStatement,
    PyEpisodicMemory,
    PyGraphCollection,
    PyGraphSchema,
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


def _compile_like_pattern(pattern: str, case_insensitive: bool) -> Any:
    import re

    parts: List[str] = []
    for ch in pattern:
        if ch == "%":
            parts.append(".*")
        elif ch == "_":
            parts.append(".")
        else:
            parts.append(re.escape(ch))
    regex = "^" + "".join(parts) + "$"
    flags = re.IGNORECASE if case_insensitive else 0
    return re.compile(regex, flags)


def _extract_filter_condition(
    filter_dict: Dict[str, Any],
) -> Optional[tuple[str, str, str]]:
    """Return (cond_type, field, pattern) from a filter dict, or None if invalid."""
    condition = filter_dict.get("condition")
    if not isinstance(condition, dict):
        return None
    cond_type = str(condition.get("type", "")).lower()
    field = condition.get("field")
    if not isinstance(field, str):
        return None
    pattern = condition.get("pattern")
    if not isinstance(pattern, str):
        return None
    return cond_type, field, pattern


def _apply_string_condition(
    value: str, cond_type: str, pattern: str
) -> bool:
    """Evaluate a LIKE/ILIKE condition against a string value."""
    if cond_type == "like":
        return bool(_compile_like_pattern(pattern, False).match(value))
    if cond_type == "ilike":
        return bool(_compile_like_pattern(pattern, True).match(value))
    return True


def _matches_filter(payload: Any, filter_dict: Optional[Dict[str, Any]]) -> bool:
    if not filter_dict:
        return True
    condition_parts = _extract_filter_condition(filter_dict)
    if condition_parts is None:
        return True
    cond_type, field, pattern = condition_parts
    if not isinstance(payload, dict):
        return False
    value = payload.get(field)
    if not isinstance(value, str):
        return False
    return _apply_string_condition(value, cond_type, pattern)


class GraphStore:
    """Compatibility adapter for GraphStore call shapes."""

    def __init__(self, inner: Optional[_RawGraphStore] = None) -> None:
        self._inner = inner or _RawGraphStore()

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def add_edge(
        self,
        edge: Any,
        *,
        source: Optional[int] = None,
        target: Optional[int] = None,
        label: Optional[str] = None,
        properties: Optional[Dict[str, Any]] = None,
    ) -> None:
        if isinstance(edge, dict):
            self._inner.add_edge(edge)
            return

        edge_dict: Dict[str, Any] = {
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
        relationship_types: Optional[List[str]] = None,
    ) -> List[TraversalResult]:
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
        relationship_types: Optional[List[str]] = None,
    ) -> List[TraversalResult]:
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
        self._graph_store: Optional[GraphStore] = None

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def upsert(
        self,
        points_or_id: Any,
        vector: Optional[Iterable[float]] = None,
        payload: Optional[Dict[str, Any]] = None,
    ) -> int:
        if vector is None:
            return self._inner.upsert(points_or_id)

        point = {"id": int(points_or_id), "vector": list(vector)}
        if payload is not None:
            point["payload"] = payload
        return self._inner.upsert([point])

    def search(
        self,
        vector: Optional[Iterable[float]] = None,
        top_k: int = 10,
        filter: Optional[Dict[str, Any]] = None,
        sparse_vector: Optional[Any] = None,
        sparse_index_name: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        kwargs: Dict[str, Any] = {"top_k": top_k}
        if vector is not None:
            kwargs["vector"] = list(vector)
        if sparse_vector is not None:
            kwargs["sparse_vector"] = sparse_vector
        if sparse_index_name is not None:
            kwargs["sparse_index_name"] = sparse_index_name
        results = self._inner.search(**kwargs)
        if not filter:
            return results
        return [r for r in results if _matches_filter(r.get("payload"), filter)]

    def batch_search(
        self,
        queries: List[Any],
        top_k: int = 10,
    ) -> List[List[Dict[str, Any]]]:
        if queries and isinstance(queries[0], dict):
            searches = []
            for q in queries:
                vector = q.get("vector", [])
                searches.append(
                    {
                        "vector": list(vector),
                        "top_k": int(q.get("top_k", top_k)),
                    }
                )
        else:
            searches = [{"vector": list(v), "top_k": int(top_k)} for v in queries]
        return self._inner.batch_search(searches)

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
    ) -> Collection:
        col = self._inner.create_collection(name, dimension, metric, storage_mode)
        return Collection(col)

    def get_collection(self, name: str) -> Optional[Collection]:
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
    ) -> Collection:
        existing = self.get_collection(name)
        if existing is not None:
            return existing
        return self.create_collection(
            name, dimension=dimension, metric=metric, storage_mode=storage_mode
        )

    def create_metadata_collection(self, name: str) -> Collection:
        col = self._inner.create_metadata_collection(name)
        return Collection(col)


class VelesQL:
    """Compatibility wrapper for VelesQL parser API."""

    def __init__(self) -> None:
        # Legacy code instantiates VelesQL(), while current API is static-only.
        pass

    @staticmethod
    def _normalize_legacy_query(query: str) -> str:
        import re

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
        except Exception:
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
