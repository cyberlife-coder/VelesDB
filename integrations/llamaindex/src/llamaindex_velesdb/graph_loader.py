"""VelesDB Graph Loader for LlamaIndex.

Load entities and relations into VelesDB's Knowledge Graph from LlamaIndex nodes.

Example:
    >>> from llamaindex_velesdb import VelesDBVectorStore, GraphLoader
    >>> from llama_index.core.schema import TextNode
    >>>
    >>> vector_store = VelesDBVectorStore(path="./db")
    >>> loader = GraphLoader(vector_store)
    >>>
    >>> # Add nodes and edges
    >>> loader.add_node(id=1, label="PERSON", metadata={"name": "John"})
    >>> loader.add_edge(id=1, source=1, target=2, label="KNOWS")
    >>>
    >>> # Query edges
    >>> edges = loader.get_edges(label="KNOWS")
"""

from typing import Any, Dict, List, Optional, TYPE_CHECKING
import hashlib

if TYPE_CHECKING:
    from llamaindex_velesdb.vectorstore import VelesDBVectorStore


def _generate_id(name: str, entity_type: str) -> int:
    """Generate a deterministic ID from entity name and type."""
    hash_input = f"{entity_type}:{name}".encode("utf-8")
    return int(hashlib.sha256(hash_input).hexdigest()[:15], 16)


class GraphLoader:
    """Load entities and relations into VelesDB's Knowledge Graph.

    Provides methods to add nodes and edges to a VelesDB collection's
    graph layer, enabling Knowledge Graph construction from LlamaIndex data.

    Args:
        vector_store: VelesDBVectorStore instance.

    Example:
        >>> loader = GraphLoader(vector_store)
        >>> loader.add_node(id=1, label="PERSON", metadata={"name": "John"})
        >>> loader.add_edge(id=1, source=1, target=2, label="KNOWS")
        >>> edges = loader.get_edges(label="KNOWS")
    """

    def __init__(self, vector_store: "VelesDBVectorStore") -> None:
        """Initialize GraphLoader with a VelesDBVectorStore."""
        self._vector_store = vector_store

    def add_node(
        self,
        id: int,
        label: str,
        metadata: Optional[Dict[str, Any]] = None,
        vector: Optional[List[float]] = None,
    ) -> None:
        """Add a node to the graph.

        Args:
            id: Unique node ID.
            label: Node label (type, e.g., "PERSON", "DOCUMENT").
            metadata: Optional node properties.
            vector: Optional embedding vector for the node.

        Example:
            >>> loader.add_node(
            ...     id=1,
            ...     label="PERSON",
            ...     metadata={"name": "John", "age": 30}
            ... )
        """
        collection = self._get_collection()
        if collection is None:
            raise ValueError("Collection not initialized")

        payload = {"label": label, **(metadata or {})}

        if vector:
            collection.upsert([{
                "id": id,
                "vector": vector,
                "payload": payload,
            }])
        else:
            collection.add_node(id=id, label=label, metadata=metadata or {})

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
        collection = self._get_collection()
        if collection is None:
            raise ValueError("Collection not initialized")

        collection.add_edge(
            id=id,
            source=source,
            target=target,
            label=label,
            metadata=metadata or {},
        )

    def get_edges(
        self,
        label: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        """Get edges from the graph.

        Args:
            label: Optional filter by edge label.

        Returns:
            List of edge dictionaries with id, source, target, label, properties.

        Example:
            >>> edges = loader.get_edges(label="KNOWS")
            >>> for edge in edges:
            ...     print(f"{edge['source']} -> {edge['target']}")
        """
        collection = self._get_collection()
        if collection is None:
            return []

        if label:
            edges = collection.get_edges_by_label(label)
        else:
            edges = collection.get_edges()

        return [
            {
                "id": e.get("id", 0),
                "source": e.get("source", 0),
                "target": e.get("target", 0),
                "label": e.get("label", ""),
                "properties": e.get("properties", {}),
            }
            for e in edges
        ]

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
            # Use deterministic SHA256-based ID (not Python hash() which is randomized)
            node_id = _generate_id(node.node_id, node_label)

            metadata = {
                "node_id": node.node_id,
                "text_preview": node.get_content()[:200] if hasattr(node, "get_content") else "",
            }

            if hasattr(node, "metadata") and node.metadata:
                metadata.update({
                    k: v for k, v in node.metadata.items()
                    if isinstance(v, (str, int, float, bool))
                })

            try:
                self.add_node(
                    id=node_id,
                    label=node_label,
                    metadata=metadata,
                )
                nodes_added += 1
            except Exception:
                pass

        return {"nodes": nodes_added, "edges": 0}

    def _get_collection(self):
        """Get the underlying collection from the vector store."""
        return self._vector_store._collection
