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
    >>> loader.add_edge(edge_id=1, source=1, target=2, label="KNOWS")
    >>>
    >>> # Query edges
    >>> edges = loader.get_edges(label="KNOWS")
"""

from typing import Any, Dict, List, Optional, Set, Tuple, TYPE_CHECKING
import logging

from velesdb_common.ids import stable_hash_id

from llamaindex_velesdb._common import open_database as _open_database

logger = logging.getLogger(__name__)

if TYPE_CHECKING:
    from llamaindex_velesdb.vectorstore import VelesDBVectorStore


def _generate_id(name: str, entity_type: str) -> int:
    """Generate a deterministic ID from entity name and type.

    Delegates to the canonical :func:`velesdb_common.ids.stable_hash_id` so all
    VelesDB integrations derive the same string->id mapping (no per-package hash).
    """
    return stable_hash_id(f"{entity_type}:{name}")


def _store_db_path(vector_store: Any) -> Optional[str]:
    """The database path a vector store exposes, or ``None``."""
    for attr in ("_db_path", "_path", "path", "_data_path"):
        candidate = getattr(vector_store, attr, None)
        if candidate is not None:
            return str(candidate)
    return None


def _store_database(vector_store: Any) -> Optional[Any]:
    """The ``velesdb.Database`` holding the vector store's data.

    Reuses the store's own handle (``_get_db()``) when it exposes one: core
    refuses a second ``Database`` on a path this process already holds open
    (VELES-031), so opening one here would always fail once the store has
    touched its database. ``GraphRetriever`` resolves its graph the same way.
    Otherwise the database is opened on the store's path with the store's
    ``_config`` (``velesdb.VelesConfigOptions``) when present.
    """
    get_db = getattr(vector_store, "_get_db", None)
    if callable(get_db):
        return get_db()

    db_path = _store_db_path(vector_store)
    if db_path is None:
        logger.debug(
            "Cannot open native graph collection: vector store has no "
            "_get_db() and no _db_path / _path / path attribute."
        )
        return None
    return _open_database(db_path, getattr(vector_store, "_config", None))


def _open_native_graph(vector_store: Any, collection_name: str) -> Optional[Any]:
    """Open a native graph collection in the vector store's database.

    Args:
        vector_store: VelesDBVectorStore instance.
        collection_name: Name of the graph collection.

    Returns:
        PyGraphCollection instance, or None if unavailable.
    """
    try:
        db = _store_database(vector_store)
        if db is None:
            return None
        graph = db.get_graph_collection(collection_name)
        if graph is None:
            logger.debug(
                "Graph collection '%s' not found; edges go to the in-memory "
                "graph store instead.",
                collection_name,
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


def _related_node_ids(related: Any) -> List[str]:
    """Node ids a relationship points at: one ``RelatedNodeInfo``, or a list (``CHILD``)."""
    items = related if isinstance(related, list) else [related]
    return [item.node_id for item in items]


def _relation_edge(label: str, source_id: int, target_id: int) -> Dict[str, Any]:
    """The edge for one relationship between two loaded graph nodes.

    Its id derives from the label and both endpoints, the way the LangChain
    loader derives a relation's, so reloading the same relationship always
    names the same edge.
    """
    return {
        "id": _generate_id(f"{source_id}->{target_id}", label),
        "source": source_id,
        "target": target_id,
        "label": label,
        "properties": {},
    }


def _relation_edges(
    node: Any, source_id: int, graph_ids: Dict[str, int]
) -> Tuple[List[Dict[str, Any]], int]:
    """Edges for ``node.relationships`` whose target is in ``graph_ids``.

    Returns the edges and the number of relationships skipped because their
    target is not in ``graph_ids`` (an edge to it would dangle).
    """
    edges: List[Dict[str, Any]] = []
    skipped = 0
    for kind, related in node.relationships.items():
        for target in _related_node_ids(related):
            target_id = graph_ids.get(target)
            if target_id is None:
                skipped += 1
                logger.debug(
                    "Skipping %s relationship %s -> %s: target not loaded in this call.",
                    kind.name, node.node_id, target,
                )
                continue
            edges.append(_relation_edge(kind.name, source_id, target_id))
    return edges, skipped


class GraphLoader:
    """Load entities and relations into VelesDB's Knowledge Graph.

    Provides methods to add nodes and edges to a VelesDB collection's
    graph layer, enabling Knowledge Graph construction from LlamaIndex data.

    When ``graph_collection_name`` names an existing graph collection in the
    vector store's database, nodes and edges are written directly to that
    persistent ``PyGraphCollection`` via the native Python bindings (no REST
    server needed). Otherwise, nodes go to the vector store's collection and
    edges to an in-memory graph store.

    Args:
        vector_store: VelesDBVectorStore instance.
        graph_collection_name: Name of the native graph collection to write
            nodes and edges to. If ``None`` (or the collection does not exist),
            an in-memory graph store is used (legacy behaviour).

    Example:
        >>> loader = GraphLoader(vector_store, graph_collection_name="kg")
        >>> loader.add_node(node_id=1, label="PERSON", metadata={"name": "John"})
        >>> loader.add_edge(edge_id=1, source=1, target=2, label="KNOWS")
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

        With a native graph collection the node is upserted into it, next to
        the edges; otherwise it is stored in the vector store's collection.

        Args:
            node_id: Unique node ID.
            label: Node label (type, e.g., "PERSON", "DOCUMENT").
            metadata: Optional node properties.
            vector: Optional embedding vector for the node. A native graph
                collection created without embeddings stores the node
                without it.

        Example:
            >>> loader.add_node(
            ...     node_id=1,
            ...     label="PERSON",
            ...     metadata={"name": "John", "age": 30}
            ... )
        """
        payload = {"label": label, **(metadata or {})}
        if self._native_graph is not None:
            self._add_native_node(node_id, payload, vector)
            return

        collection = self._get_collection(dimension=len(vector) if vector else None)

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

    def _add_native_node(
        self, node_id: int, payload: Dict[str, Any], vector: Optional[List[float]]
    ) -> None:
        """Upsert a node into the native graph collection.

        Core refuses an edge whose source or target has no node in the same
        collection (``NodeNotFound``, issue #1470), so nodes go where their
        edges go: one graph collection, as the LangChain loader writes it.
        ``_labels`` is what ``MATCH (n:LABEL)`` matches on.
        """
        if not self._native_graph.has_embeddings:
            vector = None
        self._native_graph.upsert_node(
            node_id, {"_labels": [payload["label"]], **payload}, vector
        )

    def add_edge(
        self,
        edge_id: int,
        source: int,
        target: int,
        label: str,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> None:
        """Add an edge to the graph.

        Args:
            edge_id: Unique edge ID.
            source: Source node ID.
            target: Target node ID.
            label: Edge label (relationship type, e.g., "KNOWS", "WORKS_AT").
            metadata: Optional edge properties.

        Example:
            >>> loader.add_edge(
            ...     edge_id=1,
            ...     source=100,
            ...     target=200,
            ...     label="KNOWS",
            ...     metadata={"since": "2024-01-01"}
            ... )
        """
        edge = {
            "id": edge_id,
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
                logger.warning("Failed to write edge %s to native graph: %s", edge_id, exc)
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
        """Load LlamaIndex nodes as graph nodes, and optionally their relationships.

        Each node becomes a graph node whose id is derived from ``node_label``
        and its LlamaIndex ``node_id``.

        With ``extract_relations=True``, every entry of a loaded node's
        ``relationships`` (``SOURCE``, ``PREVIOUS``, ``NEXT``, ``PARENT``,
        ``CHILD``) becomes an edge from that node to its target, labelled with
        the relationship kind's name (``"NEXT"``, ...). Only targets loaded by
        the same call are linked: a relationship whose target is not among
        ``nodes`` (or failed to load) is skipped, counted in
        ``skipped_relations``, and never written as a dangling edge. To link
        across batches, load the linked nodes together. Edge ids are
        deterministic, and an edge already in the graph is left as it is, so
        loading the same nodes again writes nothing new and returns the same
        counts. An edge that fails to write is logged and not counted.

        Args:
            nodes: List of LlamaIndex BaseNode objects.
            node_label: Label to assign to all nodes. Defaults to "DOCUMENT".
            extract_relations: Write the nodes' relationships as edges.
                Defaults to ``False`` (nodes only).

        Returns:
            Dictionary with counts: ``{"nodes": n, "edges": m,
            "skipped_relations": k}`` — ``edges`` is the number of distinct
            relationship edges the graph holds for this call (written now or
            by an earlier load); both are 0 when ``extract_relations`` is
            ``False``.

        Example:
            >>> from llama_index.core.schema import TextNode
            >>> nodes = [TextNode(text="Hello", id_="1")]
            >>> counts = loader.load_from_nodes(nodes, extract_relations=True)
        """
        loaded = self._load_graph_nodes(nodes, node_label)
        counts = {"nodes": len(loaded), "edges": 0, "skipped_relations": 0}
        if extract_relations:
            counts["edges"], counts["skipped_relations"] = self._load_relations(loaded)
        return counts

    def _load_graph_nodes(
        self, nodes: List[Any], node_label: str
    ) -> List[Tuple[Any, int]]:
        """Add each node to the graph; return ``(node, graph id)`` for those added."""
        loaded: List[Tuple[Any, int]] = []
        for node in nodes:
            node_id = _generate_id(node.node_id, node_label)
            metadata = _extract_graph_metadata(node)

            try:
                self.add_node(node_id=node_id, label=node_label, metadata=metadata)
                loaded.append((node, node_id))
            except Exception as e:
                logger.warning(f"Failed to add node {node.node_id}: {e}")
        return loaded

    def _load_relations(self, loaded: List[Tuple[Any, int]]) -> Tuple[int, int]:
        """Write the relationships among ``loaded`` nodes as edges.

        Returns (distinct edges the graph holds, relationships skipped).
        """
        graph_ids = {node.node_id: graph_id for node, graph_id in loaded}
        mirrored = {edge["id"] for edge in self._edges}
        held: Set[int] = set()
        skipped = 0
        for node, graph_id in loaded:
            edges, node_skipped = _relation_edges(node, graph_id, graph_ids)
            skipped += node_skipped
            held.update(e["id"] for e in edges if self._ensure_edge(e, mirrored))
        if skipped:
            logger.info(
                "Skipped %d relationship(s) whose target was not loaded in this call.",
                skipped,
            )
        return len(held), skipped

    def _ensure_edge(self, edge: Dict[str, Any], mirrored: Set[int]) -> bool:
        """Write ``edge`` unless the graph already holds its id.

        ``mirrored`` is the set of edge ids in the in-memory list ``get_edges``
        reads without a native graph; it keeps that list free of duplicates.
        Returns True when the graph holds the edge, False if writing it failed.
        """
        store = self._native_graph if self._native_graph is not None else self._graph_store
        try:
            if store is not None and not store.has_edge(edge["id"]):
                store.add_edge(edge)
        except Exception as exc:
            logger.warning("Failed to write %s edge %s: %s", edge["label"], edge["id"], exc)
            return False
        if edge["id"] not in mirrored:
            self._edges.append(edge)
            mirrored.add(edge["id"])
        return True

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
