"""Load extracted entities and relations into VelesDB.

Stores extracted entities as nodes and relations as edges of one persistent
VelesDB graph collection.
"""

from typing import List, Optional, Dict, TYPE_CHECKING
import logging

from velesdb_common.ids import stable_hash_id

if TYPE_CHECKING:
    from velesdb import Database, GraphCollection
    from langchain_velesdb.graph_toolkit.extractor import ExtractionResult

from langchain_velesdb.graph_toolkit.extractor import Entity, Relation

logger = logging.getLogger(__name__)


def _generate_id(name: str, entity_type: str) -> int:
    """Generate a deterministic ID from entity name and type.

    Delegates to the canonical :func:`velesdb_common.ids.stable_hash_id` so all
    VelesDB integrations derive the same string->id mapping (no per-package hash).
    """
    return stable_hash_id(f"{entity_type}:{name}")


class GraphLoader:
    """Loads extracted entities and relations into a VelesDB graph collection.

    Entities become nodes (their type is the node's label, so VelesQL
    ``MATCH (n:TYPE)`` finds them, and their embedding, when there is one, is
    the node's vector) and relations become edges, all in one persistent
    graph collection.

    Args:
        db: VelesDB Database instance.
        collection_name: Name of the graph collection (default: "knowledge_graph").
            An existing graph collection of that name is reused; otherwise one
            is created on the first load, with the dimension of the first
            embedding, or without embeddings when ``embedding_fn`` is not set.
        embedding_fn: Optional function to generate embeddings for entities.

    Example:
        >>> from velesdb import Database
        >>> db = Database("./my_graph")
        >>> loader = GraphLoader(db)
        >>> loader.load(entities, relations)
    """

    def __init__(
        self,
        db: "Database",
        collection_name: str = "knowledge_graph",
        embedding_fn: Optional[callable] = None,
    ):
        self.db = db
        self.collection_name = collection_name
        self.embedding_fn = embedding_fn
        self._graph: Optional["GraphCollection"] = None

    def _graph_collection(self, dimension: Optional[int]) -> "GraphCollection":
        """The graph collection the nodes and edges go to, created on first use.

        A failure to get or create it is raised, not logged: without it no
        entity or relation can be stored.
        """
        if self._graph is None:
            self._graph = self.db.get_graph_collection(self.collection_name)
        if self._graph is None:
            self._graph = self.db.create_graph_collection(
                self.collection_name, dimension=dimension
            )
        return self._graph

    def load(
        self,
        entities: List[Entity],
        relations: List[Relation],
        generate_embeddings: bool = True,
    ) -> Dict[str, int]:
        """Load entities and relations into VelesDB.

        Args:
            entities: List of Entity objects to load as nodes.
            relations: List of Relation objects to load as edges.
            generate_embeddings: Whether to generate embeddings for entities.

        Returns:
            Dictionary with counts: {"nodes": n, "edges": m}.
        """
        entity_map: Dict[str, int] = {}
        nodes_added = 0

        for entity in entities:
            entity_id = _generate_id(entity.name, entity.entity_type)
            if self._load_entity(entity, entity_id, generate_embeddings):
                entity_map[entity.name] = entity_id
                nodes_added += 1

        edges_added = self._load_edges(relations, entity_map)
        return {"nodes": nodes_added, "edges": edges_added}

    def _load_entity(
        self, entity: Entity, entity_id: int, generate_embeddings: bool
    ) -> bool:
        """Load a single entity as a graph node. Returns True on success."""
        # embedding_fn errors (service down, OOM) propagate to caller intentionally.
        vector = None
        if generate_embeddings and self.embedding_fn:
            vector = self.embedding_fn(f"{entity.entity_type}: {entity.name}")

        graph = self._graph_collection(len(vector) if vector is not None else None)
        if not graph.has_embeddings:
            vector = None
        payload = {
            "_labels": [entity.entity_type],
            "name": entity.name,
            "type": entity.entity_type,
            **entity.properties,
        }
        try:
            graph.upsert_node(entity_id, payload, vector)
            return True
        except Exception as exc:
            logger.warning("Failed to load entity '%s' into graph collection: %s", entity.name, exc)
            return False

    def _load_edges(
        self, relations: List[Relation], entity_map: Dict[str, int]
    ) -> int:
        """Load relations as edges between loaded nodes. Returns count added."""
        edges_added = 0
        for relation in relations:
            source_id = entity_map.get(relation.source)
            target_id = entity_map.get(relation.target)
            if source_id is None or target_id is None:
                continue
            edge_id = _generate_id(f"{relation.source}->{relation.target}", relation.relation_type)
            try:
                self._graph_collection(None).add_edge({
                    "id": edge_id, "source": source_id, "target": target_id,
                    "label": relation.relation_type, "properties": relation.properties,
                })
                edges_added += 1
            except Exception as exc:
                logger.warning(
                    "Failed to add relation '%s' -> '%s' (%s): %s",
                    relation.source, relation.target, relation.relation_type, exc,
                )
        return edges_added

    def load_from_result(
        self,
        result: "ExtractionResult",
        generate_embeddings: bool = True,
    ) -> Dict[str, int]:
        """Load directly from an ExtractionResult.

        Args:
            result: ExtractionResult from GraphExtractor.
            generate_embeddings: Whether to generate embeddings.

        Returns:
            Dictionary with counts.
        """
        from langchain_velesdb.graph_toolkit.extractor import ExtractionResult

        if not isinstance(result, ExtractionResult):
            raise TypeError("Expected ExtractionResult")

        return self.load(
            result.entities,
            result.relations,
            generate_embeddings=generate_embeddings,
        )
