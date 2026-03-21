"""VelesDB Graph Loader for LlamaIndex.

Load entities and relations into VelesDB's Knowledge Graph from LlamaIndex nodes.

Example:
    >>> from llamaindex_velesdb import VelesDBVectorStore, GraphLoader
    >>> from llama_index.core.schema import TextNode
    >>>
    >>> vector_store = VelesDBVectorStore(path="./db")
    >>>
    >>> # Native mode — edges written directly to a persistent graph collection
    >>> loader = GraphLoader(vector_store, graph_collection_name="kg")
    >>>
    >>> # Add nodes and edges
    >>> loader.add_node(node_id=1, label="PERSON", metadata={"name": "John"})
    >>> loader.add_edge(id=1, source=1, target=2, label="KNOWS")
    >>>
    >>> # Query edges
    >>> edges = loader.get_edges(label="KNOWS")
"""

from typing import Any, Dict, List, Optional, TYPE_CHECKING
import hashlib
import logging

logger = logging.getLogger(__name__)

if TYPE_CHECKING:
    from llamaindex_velesdb.vectorstore import VelesDBVectorStore


def _generate_id(name: str, entity_type: str) -> int:
    """Generate a deterministic ID from entity name and type."""
    hash_input = f"{entity_type}:{name}".encode("utf-8")
    return int(hashlib.sha256(hash_input).hexdigest()[:15], 16)


def _open_native_graph(vector_store: Any, collection_name: str) -> Optional[Any]:
    """Open a native graph collection from the vector store's database path.

    Args:
        vector_store: VelesDBVectorStore instance.
        collection_name: Name of the graph collection.

    Returns:
        PyGraphCollection instance, or None if unavailable.
    """
    db_path: Optional[str] = None
    for attr in ("_db_path", "_path", "_data_path"):
        candidate = getattr(vector_store, attr, None)
        if candidate is not None:
            db_path = str(candidate)
            break

    if db_path is None:
        logger.debug(
            "Cannot open native graph collection: vector store has no "
            "_db_path / _path attribute."
        )
        return None

    try:
        import velesdb  # type: ignore[import]
        db = velesdb.Database(db_path)
        graph = db.get_graph_collection(collection_name)
        if graph is None:
            logger.debug(
                "Graph collection '%s' not found at '%s'; "
                "it will be created on first write.",
                collection_name,
                db_path,
            )
        return graph
    except ImportError:
        logger.debug("velesdb package not installed; native graph unavailable.")
        return None
    except Exception as exc:
        logger.debug("Failed to open native graph collection: %s", exc)
        return None


def _normalise_edge(raw: Any) -> Dict[str, Any]:
    """Normalise a raw edge dict to the canonical format.

    Both the native graph and the in-memory list may return edges that are
    missing optional keys.  This function fills every key with a safe default
    so callers never have to guard against ``KeyError``.

    Args:
        raw: Edge mapping as returned by the native graph or stored in
            ``self._edges``.

    Returns:
        Dict with guaranteed keys: id, source, target, label, properties.
    """
    return {
        "id": raw.get("id", 0),
        "source": raw.get("source", 0),
        "target": raw.get("target", 0),
        "label": raw.get("label", ""),
        "properties": raw.get("properties", {}),
    }


def _extract_graph_metadata(node: Any) -> Dict[str, Any]:
    """Build metadata dict from a LlamaIndex node for graph loading."""
    content = node.get_content() if hasattr(node, "get_content") else None
    metadata: Dict[str, Any] = {
        "node_id": node.node_id,
        "text_preview": content[:200] if content else "",
    }
    if hasattr(node, "metadata") and node.metadata:
        metadata.update({
            k: v for k, v in node.metadata.items()
            if isinstance(v, (str, int, float, bool))
        })
    return metadata


class GraphLoader:
    """Load entities and relations into VelesDB's Knowledge Graph.

    Provides methods to add nodes and edges to a VelesDB collection's
    graph layer, enabling Knowledge Graph construction from LlamaIndex data.

    When ``graph_collection_name`` is provided, edges are written directly to
    the persistent ``PyGraphCollection`` via the native Python bindings (no REST
    server needed). Otherwise, the loader falls back to an in-memory graph store.

    Args:
        vector_store: VelesDBVectorStore instance.
        graph_collection_name: Name of the native graph collection to write edges
            to. If ``None``, an in-memory graph store is used (legacy behaviour).

    Example:
        >>> loader = GraphLoader(vector_store, graph_collection_name="kg")
        >>> loader.add_node(node_id=1, label="PERSON", metadata={"name": "John"})
        >>> loader.add_edge(id=1, source=1, target=2, label="KNOWS")
        >>> edges = loader.get_edges(label="KNOWS")
    """

    def __init__(
        self,
        vector_store: "VelesDBVectorStore",
        graph_collection_name: Optional[str] = None,
    ) -> None:
        """Initialize GraphLoader with a VelesDBVectorStore."""
        self._vector_store = vector_store
        self._default_dimension = 384
        self._edges: List[Dict[str, Any]] = []

        # Prefer native graph collection when a name is provided.
        self._native_graph: Optional[Any] = None
        if graph_collection_name is not None:
            self._native_graph = _open_native_graph(vector_store, graph_collection_name)

        # Legacy in-memory graph store fallback.
        self._graph_store = getattr(vector_store, "_graph_store", None)
        if self._graph_store is None and self._native_graph is None:
            try:
                import velesdb

                self._graph_store = velesdb.GraphStore()
            except Exception:
                self._graph_store = None

    def add_node(
        self,
        node_id: int,
        label: str,
        metadata: Optional[Dict[str, Any]] = None,
        vector: Optional[List[float]] = None,
    ) -> None:
        """Add a node to the graph.

        Args:
            node_id: Unique node ID.
            label: Node label (type, e.g., "PERSON", "DOCUMENT").
            metadata: Optional node properties.
            vector: Optional embedding vector for the node.

        Example:
            >>> loader.add_node(
            ...     node_id=1,
            ...     label="PERSON",
            ...     metadata={"name": "John", "age": 30}
            ... )
        """
        collection = self._get_collection(dimension=len(vector) if vector else None)

        payload = {"label": label, **(metadata or {})}

        if vector is not None:
            collection.upsert([{
                "id": node_id,
                "vector": vector,
                "payload": payload,
            }])
        else:
            collection.upsert_metadata([{
                "id": node_id,
                "payload": payload,
            }])

    def add_edge(
        self,
        id: int,
        source: int,
        target: int,
        label: str,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> None:
        """Add an edge to the graph.

        Args:
            id: Unique edge ID.
            source: Source node ID.
            target: Target node ID.
            label: Edge label (relationship type, e.g., "KNOWS", "WORKS_AT").
            metadata: Optional edge properties.

        Example:
            >>> loader.add_edge(
            ...     id=1,
            ...     source=100,
            ...     target=200,
            ...     label="KNOWS",
            ...     metadata={"since": "2024-01-01"}
            ... )
        """
        edge = {
            "id": id,
            "source": source,
            "target": target,
            "label": label,
            "properties": metadata or {},
        }
        self._edges.append(edge)

        # Write to persistent native graph collection when available.
        if self._native_graph is not None:
            try:
                self._native_graph.add_edge(edge)
            except Exception as exc:
                logger.warning("Failed to write edge %s to native graph: %s", id, exc)
        elif self._graph_store is not None:
            self._graph_store.add_edge(edge)

    def get_edges(
        self,
        label: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        """Get edges from the graph.

        When a native graph collection is available it is used as the source of
        truth (ensuring cross-session persistence).  If the native graph raises
        an exception the method falls back to the in-memory list and logs a
        warning.  Without a native graph the in-memory list is used directly
        (legacy behaviour).

        Args:
            label: Optional filter by edge label.  Forwarded to the native graph
                when available so filtering happens at the storage layer.

        Returns:
            List of edge dictionaries with id, source, target, label, properties.

        Example:
            >>> edges = loader.get_edges(label="KNOWS")
            >>> for edge in edges:
            ...     print(f"{edge['source']} -> {edge['target']}")
        """
        if self._native_graph is not None:
            try:
                raw_edges = self._native_graph.get_edges(label)
                return [_normalise_edge(e) for e in raw_edges]
            except Exception as exc:
                logger.warning(
                    "Failed to read edges from native graph (%s); "
                    "falling back to in-memory list.",
                    exc,
                )

        # In-memory fallback (no native graph, or native graph failed).
        if label:
            source = [e for e in self._edges if e.get("label") == label]
        else:
            source = list(self._edges)

        return [_normalise_edge(e) for e in source]

    def load_from_nodes(
        self,
        nodes: List[Any],
        node_label: str = "DOCUMENT",
        extract_relations: bool = False,
    ) -> Dict[str, int]:
        """Load LlamaIndex nodes as graph nodes.

        Args:
            nodes: List of LlamaIndex BaseNode objects.
            node_label: Label to assign to all nodes. Defaults to "DOCUMENT".
            extract_relations: Whether to extract relations (requires NLP).

        Returns:
            Dictionary with counts: {"nodes": n, "edges": m}.

        Example:
            >>> from llama_index.core.schema import TextNode
            >>> nodes = [TextNode(text="Hello", id_="1")]
            >>> counts = loader.load_from_nodes(nodes)
        """
        nodes_added = 0

        for node in nodes:
            node_id = _generate_id(node.node_id, node_label)
            metadata = _extract_graph_metadata(node)

            try:
                self.add_node(node_id=node_id, label=node_label, metadata=metadata)
                nodes_added += 1
            except Exception as e:
                logger.warning(f"Failed to add node {node.node_id}: {e}")

        return {"nodes": nodes_added, "edges": 0}

    def _get_collection(self, dimension: Optional[int] = None):
        """Get or lazily initialize the underlying collection."""
        collection = getattr(self._vector_store, "_collection", None)
        if collection is not None:
            return collection

        get_collection = getattr(self._vector_store, "_get_collection", None)
        if get_collection is None:
            raise ValueError(
                "Vector store does not expose _get_collection; cannot initialize collection."
            )

        resolved_dimension = dimension or getattr(self._vector_store, "_dimension", None)
        if resolved_dimension is None:
            resolved_dimension = self._default_dimension

        collection = get_collection(int(resolved_dimension))
        if collection is None:
            raise ValueError("Failed to initialize collection from vector store")
        return collection
