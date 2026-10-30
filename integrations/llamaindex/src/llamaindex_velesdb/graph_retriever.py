"""VelesDB Graph Retriever for LlamaIndex.

Provides a retriever that combines vector search with graph traversal
for context expansion in RAG applications.

Example:
    >>> from llamaindex_velesdb import VelesDBVectorStore, GraphRetriever
    >>> from llama_index.core import VectorStoreIndex
    >>>
    >>> vector_store = VelesDBVectorStore(path="./db", collection="docs")
    >>> index = VectorStoreIndex.from_vector_store(vector_store)
    >>>
    >>> # Native mode (no server required)
    >>> retriever = GraphRetriever(
    ...     index=index,
    ...     mode="native",
    ...     graph_collection_name="my_graph",
    ...     max_depth=2
    ... )
    >>>
    >>> # REST mode (backward-compatible)
    >>> retriever = GraphRetriever(
    ...     index=index,
    ...     mode="rest",
    ...     server_url="http://localhost:8080",
    ...     max_depth=2
    ... )
    >>>
    >>> nodes = retriever.retrieve("What is machine learning?")
"""

from typing import Any, List, Optional, Tuple
import logging

from llamaindex_velesdb._common import (
    build_graph_rest_payload,
    is_timeout_exception,
    open_native_graph,
    parse_graph_traverse_results,
    point_id,
)

# Lazy VelesDBError resolution. `velesdb` is a runtime dependency for
# every user of this integration (they pass a `velesdb.Collection`
# instance to the retriever), but the integration package itself is
# importable without it (e.g. for documentation, type-stubs, or smoke
# tests running without the compiled Rust extension). When the typed
# exception hierarchy is available we catch it alongside the built-in
# errors; otherwise we fall back to `Exception` which never breaks the
# defensive `try`/`except` logic below.
try:
    from velesdb import VelesDBError as _VelesDBError
except (ImportError, AttributeError):  # pragma: no cover — optional dependency fallback
    _VelesDBError = Exception  # type: ignore[misc,assignment]

logger = logging.getLogger(__name__)

try:
    from llama_index.core.retrievers import BaseRetriever
    from llama_index.core.schema import NodeWithScore, QueryBundle, TextNode
except ImportError:
    # Fallback for older llama-index versions
    from llama_index.retrievers import BaseRetriever
    from llama_index.schema import NodeWithScore, QueryBundle, TextNode


def _infer_db_path(index: Any) -> Optional[str]:
    """Try to infer the database path from an index's vector store."""
    try:
        vs = index._vector_store
        for attr in ("_db_path", "_path", "_data_path"):
            path = getattr(vs, attr, None)
            if path is not None:
                return str(path)
    except AttributeError as exc:
        logger.debug("Failed to infer db_path from index: %s", exc)
    return None


class GraphRetriever(BaseRetriever):
    """Retriever that uses graph traversal for context expansion.

    This retriever implements the "seed + expand" pattern:
    1. Vector search to find initial seed nodes
    2. Graph traversal to expand context to related nodes
    3. Return combined results for RAG

    Results: the ``seed_k`` vector hits first, best score first, then the
    nodes the graph reaches from them — seed by seed, each in traversal
    order — skipping any already returned or that the vector store cannot
    return, until ``expand_k`` nodes in all. Seeds carry ``graph_depth=0``;
    a neighbour carries ``graph_depth`` = its hops from the nearest seed, as
    the traversal reports it, and ``score=None``: it was reached through the
    graph, not scored. With ``low_latency=True`` only the seeds are returned.

    Supports two modes:

    - ``"native"`` (default): uses the VelesDB Python bindings directly,
      no server required. Requires either ``db_path`` or an index whose
      vector store exposes ``_db_path`` / ``_path``, plus
      ``graph_collection_name``.
    - ``"rest"``: legacy HTTP mode — calls the VelesDB REST server.
      Requires ``server_url``.

    Args:
        index: VectorStoreIndex with VelesDBVectorStore.
        mode: Operation mode, ``"native"`` or ``"rest"`` (default: ``"native"``).
        db_path: Filesystem path to VelesDB data directory (native mode).
        graph_collection_name: Name of the graph collection (native mode).
        server_url: URL of VelesDB server (REST mode only).
        seed_k: Number of vector search hits the graph is expanded from
            (default: 3).
        expand_k: Maximum nodes returned, seeds included (default: 10).
        max_depth: Maximum graph traversal depth (default: 2).
        rel_types: Relationship types to follow (default: all).
        collection_name: Override collection name for REST traversal URL.
        low_latency: If True, skip graph expansion for minimal latency (default: False).
        timeout_ms: Timeout for REST graph operations in milliseconds (default: 1000).
        fallback_on_timeout: If True, return vector-only results on timeout (default: True).

    Example:
        >>> # Native mode — preferred
        >>> retriever = GraphRetriever(
        ...     index=index,
        ...     mode="native",
        ...     graph_collection_name="my_graph",
        ...     seed_k=3,
        ...     expand_k=10,
        ...     max_depth=2
        ... )
        >>>
        >>> # REST mode — backward-compatible
        >>> retriever = GraphRetriever(
        ...     index=index,
        ...     mode="rest",
        ...     server_url="http://localhost:8080",
        ...     seed_k=3,
        ...     expand_k=10,
        ...     max_depth=2
        ... )
        >>>
        >>> # Minimal latency mode (vector search only)
        >>> fast_retriever = GraphRetriever(
        ...     index=index,
        ...     low_latency=True
        ... )
    """

    def __init__(
        self,
        index: Any,  # VectorStoreIndex
        mode: str = "native",
        db_path: Optional[str] = None,
        graph_collection_name: Optional[str] = None,
        server_url: str = "http://localhost:8080",
        seed_k: int = 3,
        expand_k: int = 10,
        max_depth: int = 2,
        rel_types: Optional[List[str]] = None,
        collection_name: Optional[str] = None,
        low_latency: bool = False,
        timeout_ms: int = 1000,
        fallback_on_timeout: bool = True,
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)

        if mode not in ("native", "rest"):
            raise ValueError(
                f"Invalid mode '{mode}'. Must be 'native' or 'rest'."
            )

        from llamaindex_velesdb.security import validate_k
        validate_k(seed_k, "seed_k")
        validate_k(expand_k, "expand_k")

        if mode == "rest":
            from llamaindex_velesdb.security import validate_url
            validate_url(server_url)

        self._index = index
        self._mode = mode
        self._server_url = server_url
        self._seed_k = seed_k
        self._expand_k = expand_k
        self._max_depth = max_depth
        self._rel_types = rel_types or []
        self._collection_name = collection_name or self._infer_collection_name()
        self._low_latency = low_latency
        self._timeout_ms = timeout_ms
        self._fallback_on_timeout = fallback_on_timeout
        self._graph_collection: Any = None

        # low_latency mode never traverses the graph, so it must not
        # require a graph collection to exist (vector-only retrieval).
        if mode == "native" and not low_latency:
            self._graph_collection = self._init_native_graph(
                db_path, graph_collection_name
            )

    def _init_native_graph(
        self, db_path: Optional[str], collection_name: Optional[str]
    ) -> Any:
        """Resolve the shared Database and open the native graph collection.

        Reuses the index's vector store lazy Database instance (via
        ``_get_db()``) instead of opening a second ``velesdb.Database`` on
        the same path, which would trigger VELES-031 (exclusive write-lock).

        Falls back to path-based open when the vector store does not expose
        ``_get_db`` (e.g. REST-only stores or custom implementations).
        """
        if not collection_name:
            raise ValueError(
                "Native mode requires 'graph_collection_name'."
            )
        try:
            vs = self._index._vector_store
            get_db = getattr(vs, "_get_db", None)
        except AttributeError:
            get_db = None

        if get_db is not None:
            return self._open_graph_from_shared_db(get_db, collection_name)
        return self._open_graph_from_path(db_path, collection_name)

    def _open_graph_from_shared_db(
        self, get_db: Any, collection_name: str
    ) -> Any:
        """Open graph collection from the vector store's shared Database."""
        db = get_db()
        graph = db.get_graph_collection(collection_name)
        if graph is None:
            raise ValueError(
                f"Graph collection '{collection_name}' not found "
                "in the shared database."
            )
        return graph

    def _open_graph_from_path(
        self, db_path: Optional[str], collection_name: str
    ) -> Any:
        """Open graph collection by resolving db_path (fallback path).

        Reuses the vector store's ``_config`` (``velesdb.VelesConfigOptions``)
        when present — the same source ``_get_db()`` uses — so the graph
        database is opened with the same engine configuration.
        """
        resolved_path = db_path or _infer_db_path(self._index)
        if resolved_path is None:
            raise ValueError(
                "Native mode requires 'db_path' or an index whose vector store "
                "exposes a '_db_path' / '_path' attribute."
            )
        try:
            vs = self._index._vector_store
        except AttributeError:
            vs = None
        config = getattr(vs, "_config", None)
        if config is None:
            # Keep the pre-config call shape so a velesdb-common that
            # predates open_native_graph's ``config`` parameter (floor
            # 3.8.0) stays compatible when the feature is unused.
            return open_native_graph(resolved_path, collection_name)
        return open_native_graph(resolved_path, collection_name, config=config)

    def _infer_collection_name(self) -> str:
        """Try to infer collection name from the index's vector store."""
        try:
            vs = self._index._vector_store
            if hasattr(vs, "_collection_name"):
                return vs._collection_name
            if hasattr(vs, "collection_name"):
                return vs.collection_name
        except AttributeError as exc:
            logger.debug("Failed to infer collection name from vector store: %s", exc)
        return "default"

    def _retrieve(self, query_bundle: QueryBundle) -> List[NodeWithScore]:
        """Retrieve nodes using vector search + graph expansion.

        Args:
            query_bundle: Query bundle with query string.

        Returns:
            List of NodeWithScore objects.
        """
        query_str = query_bundle.query_str
        base_retriever = self._index.as_retriever(similarity_top_k=self._seed_k)
        seed_nodes = base_retriever.retrieve(query_str)

        if not seed_nodes:
            return []

        if self._low_latency:
            return self._build_vector_only_results(seed_nodes)

        return self._build_expanded_results(seed_nodes)

    def _build_vector_only_results(
        self, seed_nodes: List[NodeWithScore]
    ) -> List[NodeWithScore]:
        """Return seed nodes annotated for vector-only mode."""
        for nws in seed_nodes[: self._expand_k]:
            nws.node.metadata["graph_depth"] = 0
            nws.node.metadata["retrieval_mode"] = "vector_only"
        return seed_nodes[: self._expand_k]

    def _build_expanded_results(
        self, seed_nodes: List[NodeWithScore]
    ) -> List[NodeWithScore]:
        """Run graph expansion and return combined node list.

        ``seed_map`` and ``neighbour_ids`` are dicts used as insertion-ordered
        sets: seeds keep their vector-search order, and neighbours the order
        they were reached in (seed by seed, each seed's traversal in order).
        """
        neighbour_ids: dict = {}
        seed_map: dict = {}
        graph_available = True

        for nws in seed_nodes:
            node_id = self._extract_node_id(nws.node)
            if node_id is None:
                continue
            seed_map[node_id] = nws

            if graph_available:
                graph_available = self._expand_from_seed(
                    node_id, neighbour_ids, graph_available
                )

        return self._assemble_results(seed_map, neighbour_ids, graph_available)

    def _expand_from_seed(
        self, node_id: int, neighbour_ids: dict, graph_available: bool
    ) -> bool:
        """Traverse graph from one seed node; return updated graph_available flag.

        Reached ids are added to ``neighbour_ids`` (id -> depth) in traversal
        order. An id already there keeps its first position and the smaller
        depth, so a neighbour's depth counts hops from the nearest seed.
        """
        try:
            for neighbor_id, depth in self._traverse_graph(node_id):
                neighbour_ids[neighbor_id] = min(depth, neighbour_ids.get(neighbor_id, depth))
        except (ValueError, RuntimeError, OSError, TimeoutError, ConnectionError, _VelesDBError) as exc:
            if self._is_timeout(exc):
                logger.warning(
                    "Graph traversal timeout for node %s, falling back to vector-only",
                    node_id,
                )
                if self._fallback_on_timeout:
                    return False
                raise
            logger.debug("Graph traversal failed for node %s: %s", node_id, exc)
        return graph_available

    def _is_timeout(self, exc: Exception) -> bool:
        """Return True if the exception represents a network/operation timeout."""
        return is_timeout_exception(exc)

    def _assemble_results(
        self,
        seed_map: dict,
        neighbour_ids: dict,
        graph_available: bool,
    ) -> List[NodeWithScore]:
        """Seeds in vector-search order, then neighbours, at most ``expand_k``."""
        retrieval_mode = "graph_expanded" if graph_available else "vector_fallback"
        results: List[NodeWithScore] = []

        for _node_id, nws in seed_map.items():
            nws.node.metadata["graph_depth"] = 0
            nws.node.metadata["retrieval_mode"] = retrieval_mode
            results.append(nws)

        results = results[: self._expand_k]
        if graph_available:
            self._append_neighbor_nodes(results, neighbour_ids, seed_map)
        return results

    def _append_neighbor_nodes(
        self, results: List[NodeWithScore], neighbour_ids: dict, seed_map: dict
    ) -> None:
        """Append neighbours in reach order until ``expand_k`` results.

        A neighbour that is a seed is skipped, and so is one the vector store
        cannot return; neither takes a slot. Neighbours carry ``score=None``:
        they were reached through the graph, not scored.
        """
        for neighbor_id, depth in neighbour_ids.items():
            if len(results) >= self._expand_k:
                return
            if neighbor_id in seed_map:
                continue
            try:
                neighbor_node = self._fetch_node(neighbor_id)
                if neighbor_node:
                    neighbor_node.metadata["graph_depth"] = depth
                    neighbor_node.metadata["retrieval_mode"] = "graph_expanded"
                    results.append(NodeWithScore(node=neighbor_node, score=None))
            except (ValueError, RuntimeError, OSError, KeyError, ConnectionError, _VelesDBError) as exc:
                logger.debug("Failed to fetch neighbour node %s: %s", neighbor_id, exc)

    def _extract_node_id(self, node: Any) -> Optional[int]:
        """The graph id of a seed: the point id its node id is stored under.

        ``point_id`` is the derivation ``VelesDBVectorStore`` stores the node
        under and ``GraphLoader`` gives its native graph node, so the seed
        expands from the node the loader linked, whatever its metadata holds
        and even when its node id is a numeric string. Edges added by hand
        must use ``point_id(node.node_id)`` as ``source`` / ``target`` too.
        LangChain reads the stored point id back from ``metadata["_int_id"]``
        instead; both name the same point.
        """
        node_id = getattr(node, "node_id", None)
        return None if node_id is None else point_id(node_id)

    def _traverse_graph(self, source_id: int) -> List[Tuple[int, int]]:
        """Traverse graph from source node, dispatching to native or REST.

        Args:
            source_id: Starting node ID.

        Returns:
            ``(node_id, depth)`` pairs in traversal order, ``depth`` being
            the number of hops from ``source_id``.
        """
        if self._mode == "native":
            return self._traverse_graph_native(source_id)
        return self._traverse_graph_rest(source_id)

    def _traverse_graph_native(self, source_id: int) -> List[Tuple[int, int]]:
        """Traverse graph using the native Python bindings.

        Args:
            source_id: Starting node ID.

        Returns:
            ``(node_id, depth)`` pairs in traversal order.
        """
        results = self._graph_collection.traverse_bfs(
            source_id,
            max_depth=self._max_depth,
            limit=self._expand_k * 2,
            rel_types=self._rel_types,
        )
        return [(r["target_id"], r["depth"]) for r in results]

    def _traverse_graph_rest(self, source_id: int) -> List[Tuple[int, int]]:
        """Traverse graph via the VelesDB REST API.

        Args:
            source_id: Starting node ID.

        Returns:
            ``(node_id, depth)`` pairs in traversal order.

        Raises:
            requests.exceptions.Timeout: If the request exceeds ``timeout_ms``.
        """
        import requests

        url = f"{self._server_url}/collections/{self._collection_name}/graph/traverse"
        payload = build_graph_rest_payload(
            source_id, self._max_depth, self._expand_k, self._rel_types
        )
        timeout_sec = self._timeout_ms / 1000.0
        response = requests.post(url, json=payload, timeout=timeout_sec)
        return parse_graph_traverse_results(response)

    def _fetch_node(self, node_id: int) -> Optional[TextNode]:
        """Fetch a node by its **internal integer ID** (already hashed).

        ``node_id`` is the hash-based int returned by
        ``_extract_node_id`` / ``point_id(node_id)``. Delegates to
        :meth:`VelesDBVectorStore.get_nodes_by_int_ids`, which passes the
        ID straight to the underlying collection — using
        :meth:`~VelesDBVectorStore.get_nodes` instead would re-hash the
        int's string form and return nothing, making graph expansion a
        silent no-op.
        """
        try:
            vs = self._index._vector_store
            fetcher = getattr(vs, "get_nodes_by_int_ids", None)
            if fetcher is None:
                return None
            results = fetcher([node_id])
            return results[0] if results else None
        except (ValueError, RuntimeError, OSError, KeyError, ConnectionError, _VelesDBError) as exc:
            logger.debug("Failed to fetch node %s from vector store: %s", node_id, exc)
        return None


class GraphQARetriever(GraphRetriever):
    """Graph-enhanced retriever optimized for Q&A tasks.

    Extends GraphRetriever with:
    - Automatic re-ranking based on graph distance
    - Support for multi-hop reasoning
    """

    def __init__(
        self,
        index: Any,
        rerank_by_depth: bool = True,
        **kwargs: Any,
    ) -> None:
        super().__init__(index=index, **kwargs)
        self._rerank_by_depth = rerank_by_depth

    def _retrieve(self, query_bundle: QueryBundle) -> List[NodeWithScore]:
        """Retrieve with re-ranking."""
        nodes = super()._retrieve(query_bundle)

        if self._rerank_by_depth:
            # Sort by graph depth, then by score
            # Unscored (graph-reached) nodes keep their order after scored ones.
            nodes.sort(key=lambda n: (
                n.node.metadata.get("graph_depth", 999),
                n.score is None,
                -(n.score or 0.0),
            ))

        return nodes
