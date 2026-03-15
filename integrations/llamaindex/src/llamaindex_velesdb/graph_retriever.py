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

from typing import Any, List, Optional
from dataclasses import dataclass, field
import logging

logger = logging.getLogger(__name__)

try:
    from llama_index.core.retrievers import BaseRetriever
    from llama_index.core.schema import NodeWithScore, QueryBundle, TextNode
except ImportError:
    # Fallback for older llama-index versions
    from llama_index.retrievers import BaseRetriever
    from llama_index.schema import NodeWithScore, QueryBundle, TextNode


@dataclass
class TraversalResult:
    """Result from graph traversal."""
    target_id: int
    depth: int
    path: List[int] = field(default_factory=list)


def _open_native_graph(db_path: str, collection_name: str) -> Any:
    """Open a native graph collection from a VelesDB database.

    Args:
        db_path: Filesystem path to the VelesDB database directory.
        collection_name: Name of the graph collection to open.

    Returns:
        PyGraphCollection instance.

    Raises:
        ImportError: If the velesdb package is not installed.
        ValueError: If the graph collection cannot be found.
    """
    try:
        import velesdb  # type: ignore[import]
    except ImportError as exc:
        raise ImportError(
            "The 'velesdb' package is required for native mode. "
            "Install it with: pip install velesdb"
        ) from exc

    db = velesdb.Database(db_path)
    graph = db.get_graph_collection(collection_name)
    if graph is None:
        raise ValueError(
            f"Graph collection '{collection_name}' not found in database at '{db_path}'"
        )
    return graph


def _infer_db_path(index: Any) -> Optional[str]:
    """Try to infer the database path from an index's vector store."""
    try:
        vs = index._vector_store
        for attr in ("_db_path", "_path", "_data_path"):
            path = getattr(vs, attr, None)
            if path is not None:
                return str(path)
    except Exception as exc:
        logger.debug("Failed to infer db_path from index: %s", exc)
    return None


class GraphRetriever(BaseRetriever):
    """Retriever that uses graph traversal for context expansion.

    This retriever implements the "seed + expand" pattern:
    1. Vector search to find initial seed nodes
    2. Graph traversal to expand context to related nodes
    3. Return combined results for RAG

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
        seed_k: Number of initial vector search results (default: 3).
        expand_k: Maximum nodes to include after expansion (default: 10).
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

        if mode == "native":
            self._graph_collection = self._init_native_graph(
                db_path, graph_collection_name
            )

    def _init_native_graph(
        self, db_path: Optional[str], collection_name: Optional[str]
    ) -> Any:
        """Resolve db_path and open the native graph collection."""
        resolved_path = db_path or _infer_db_path(self._index)
        if resolved_path is None:
            raise ValueError(
                "Native mode requires 'db_path' or an index whose vector store "
                "exposes a '_db_path' / '_path' attribute."
            )
        if not collection_name:
            raise ValueError(
                "Native mode requires 'graph_collection_name'."
            )
        return _open_native_graph(resolved_path, collection_name)

    def _infer_collection_name(self) -> str:
        """Try to infer collection name from the index's vector store."""
        try:
            vs = self._index._vector_store
            if hasattr(vs, "_collection_name"):
                return vs._collection_name
            if hasattr(vs, "collection_name"):
                return vs.collection_name
        except Exception as exc:
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
        k = self._seed_k if self._low_latency else self._expand_k
        base_retriever = self._index.as_retriever(similarity_top_k=k)
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
        """Run graph expansion and return combined node list."""
        expanded_ids: set = set()
        seed_map: dict = {}
        graph_available = True

        for nws in seed_nodes:
            node_id = self._extract_node_id(nws.node)
            if node_id is None:
                continue
            seed_map[node_id] = nws
            expanded_ids.add(node_id)

            if graph_available:
                graph_available = self._expand_from_seed(
                    node_id, expanded_ids, graph_available
                )

        return self._assemble_results(seed_map, expanded_ids, graph_available)

    def _expand_from_seed(
        self, node_id: int, expanded_ids: set, graph_available: bool
    ) -> bool:
        """Traverse graph from one seed node; return updated graph_available flag."""
        try:
            neighbors = self._traverse_graph(node_id)
            for neighbor_id in neighbors:
                expanded_ids.add(neighbor_id)
        except Exception as exc:
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
        try:
            import requests
            if isinstance(exc, requests.exceptions.Timeout):
                return True
        except ImportError:
            pass
        return isinstance(exc, TimeoutError)

    def _assemble_results(
        self,
        seed_map: dict,
        expanded_ids: set,
        graph_available: bool,
    ) -> List[NodeWithScore]:
        """Build the final result list from seeds and expanded neighbours."""
        retrieval_mode = "graph_expanded" if graph_available else "vector_fallback"
        results: List[NodeWithScore] = []

        for node_id, nws in seed_map.items():
            nws.node.metadata["graph_depth"] = 0
            nws.node.metadata["retrieval_mode"] = retrieval_mode
            results.append(nws)

        if graph_available:
            remaining = self._expand_k - len(results)
            neighbor_ids = [
                nid for nid in expanded_ids if nid not in seed_map
            ][:remaining]
            for neighbor_id in neighbor_ids:
                try:
                    neighbor_node = self._fetch_node(neighbor_id)
                    if neighbor_node:
                        neighbor_node.metadata["graph_depth"] = 1
                        neighbor_node.metadata["retrieval_mode"] = "graph_expanded"
                        results.append(NodeWithScore(node=neighbor_node, score=0.5))
                except Exception as exc:
                    logger.debug(
                        "Failed to fetch neighbour node %s: %s", neighbor_id, exc
                    )

        return results[: self._expand_k]

    def _extract_node_id(self, node: Any) -> Optional[int]:
        """Extract numeric node ID from a LlamaIndex node."""
        try:
            if hasattr(node, "metadata"):
                for key in ["id", "doc_id", "node_id"]:
                    if key in node.metadata:
                        val = node.metadata[key]
                        return int(val) if isinstance(val, (int, str)) else None
            if hasattr(node, "node_id"):
                try:
                    return int(node.node_id)
                except (ValueError, TypeError):
                    pass
        except Exception as exc:
            logger.debug("Failed to extract node id from node metadata: %s", exc)
        return None

    def _traverse_graph(self, source_id: int) -> List[int]:
        """Traverse graph from source node, dispatching to native or REST.

        Args:
            source_id: Starting node ID.

        Returns:
            List of neighbour node IDs.
        """
        if self._mode == "native":
            return self._traverse_graph_native(source_id)
        return self._traverse_graph_rest(source_id)

    def _traverse_graph_native(self, source_id: int) -> List[int]:
        """Traverse graph using the native Python bindings.

        Args:
            source_id: Starting node ID.

        Returns:
            List of neighbour node IDs.
        """
        results = self._graph_collection.traverse_bfs(
            source_id,
            max_depth=self._max_depth,
            limit=self._expand_k * 2,
            rel_types=self._rel_types,
        )
        return [r["target_id"] for r in results]

    def _traverse_graph_rest(self, source_id: int) -> List[int]:
        """Traverse graph via the VelesDB REST API.

        Args:
            source_id: Starting node ID.

        Returns:
            List of neighbour node IDs.

        Raises:
            requests.exceptions.Timeout: If the request exceeds ``timeout_ms``.
        """
        import requests

        url = f"{self._server_url}/collections/{self._collection_name}/graph/traverse"
        payload = {
            "source": source_id,
            "strategy": "bfs",
            "max_depth": self._max_depth,
            "limit": self._expand_k * 2,
            "rel_types": self._rel_types,
        }

        timeout_sec = self._timeout_ms / 1000.0
        response = requests.post(url, json=payload, timeout=timeout_sec)

        if response.status_code == 200:
            data = response.json()
            return [r["target_id"] for r in data.get("results", [])]

        return []

    def _fetch_node(self, node_id: int) -> Optional[TextNode]:
        """Fetch a node by ID from the vector store.

        Args:
            node_id: Node ID.

        Returns:
            TextNode or None if not found.
        """
        try:
            vs = self._index._vector_store
            if hasattr(vs, "get_nodes"):
                results = vs.get_nodes([str(node_id)])
                if results:
                    return results[0]
        except Exception as exc:
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
            nodes.sort(key=lambda n: (
                n.node.metadata.get("graph_depth", 999),
                -n.score
            ))

        return nodes
