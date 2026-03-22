"""Load extracted entities and relations into VelesDB.

Handles conversion from extracted data to VelesDB graph storage format.
"""

from typing import List, Optional, Dict, TYPE_CHECKING
import hashlib
import logging
import velesdb

if TYPE_CHECKING:
    from velesdb import Database, Collection
    from langchain_velesdb.graph_toolkit.extractor import ExtractionResult

from langchain_velesdb.graph_toolkit.extractor import Entity, Relation

logger = logging.getLogger(__name__)


def _generate_id(name: str, entity_type: str) -> int:
    """Generate a deterministic ID from entity name and type."""
    hash_input = f"{entity_type}:{name}".encode("utf-8")
    return int(hashlib.sha256(hash_input).hexdigest()[:15], 16)


class GraphLoader:
    """Loads extracted entities and relations into VelesDB.

    Handles the conversion from extracted Entity/Relation objects
    to VelesDB Points, Nodes, and Edges.

    Args:
        db: VelesDB Database instance.
        collection_name: Name of the collection to use (default: "knowledge_graph").
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
        self._collection: Optional["Collection"] = None
        self._graph_store = getattr(db, "graph_store", None)
        if self._graph_store is None and hasattr(velesdb, "GraphStore"):
            self._graph_store = velesdb.GraphStore()
        self._metadata_only = False

    def _get_or_create_collection(self, dimension: Optional[int] = None) -> "Collection":
        """Get an existing collection or create one using real Python bindings."""
        if self._collection is None:
            existing = self.db.get_collection(self.collection_name)
            if existing is not None:
                self._collection = existing
                info = existing.info()
                self._metadata_only = bool(info.get("metadata_only", False))
                return existing

            if dimension is not None:
                self._collection = self.db.create_collection(
                    self.collection_name,
                    dimension=dimension,
                    metric="cosine",
                    storage_mode="full",
                )
                self._metadata_only = False
            else:
                self._collection = self.db.create_metadata_collection(self.collection_name)
                self._metadata_only = True
        return self._collection

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
            entity_map[entity.name] = entity_id
            if self._load_entity(entity, entity_id, generate_embeddings):
                nodes_added += 1

        edges_added = self._load_edges(relations, entity_map)
        return {"nodes": nodes_added, "edges": edges_added}

    def _load_entity(
        self, entity: Entity, entity_id: int, generate_embeddings: bool
    ) -> bool:
        """Load a single entity into the collection. Returns True on success."""
        metadata = {"name": entity.name, "type": entity.entity_type, **entity.properties}

        vector = None
        if generate_embeddings and self.embedding_fn:
            vector = self.embedding_fn(f"{entity.entity_type}: {entity.name}")

        try:
            collection = self._get_or_create_collection(
                len(vector) if vector is not None else None
            )
            payload = {"label": entity.entity_type, **metadata}
            if vector is not None and not self._metadata_only:
                collection.upsert([{"id": entity_id, "vector": vector, "payload": payload}])
            else:
                collection.upsert_metadata([{"id": entity_id, "payload": payload}])
            return True
        except Exception as exc:
            logger.warning("Failed to load entity '%s' into graph collection: %s", entity.name, exc)
            return False

    def _load_edges(
        self, relations: List[Relation], entity_map: Dict[str, int]
    ) -> int:
        """Load relations as edges into the graph store. Returns count added."""
        edges_added = 0
        for relation in relations:
            source_id = entity_map.get(relation.source)
            target_id = entity_map.get(relation.target)
            if source_id is None or target_id is None or self._graph_store is None:
                continue
            edge_id = _generate_id(f"{relation.source}->{relation.target}", relation.relation_type)
            try:
                self._graph_store.add_edge({
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
