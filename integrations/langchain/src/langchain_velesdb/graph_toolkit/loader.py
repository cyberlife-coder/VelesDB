"""Load extracted entities and relations into VelesDB.

Handles conversion from extracted data to VelesDB graph storage format.
"""

from typing import List, Optional, Dict, Any, TYPE_CHECKING
import hashlib

if TYPE_CHECKING:
    from velesdb import Database, Collection

from langchain_velesdb.graph_toolkit.extractor import Entity, Relation


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

    @property
    def collection(self) -> "Collection":
        """Get or create the collection."""
        if self._collection is None:
            self._collection = self.db.get_or_create_collection(self.collection_name)
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
        edges_added = 0

        for entity in entities:
            entity_id = _generate_id(entity.name, entity.entity_type)
            entity_map[entity.name] = entity_id

            metadata = {
                "name": entity.name,
                "type": entity.entity_type,
                **entity.properties,
            }

            vector = None
            if generate_embeddings and self.embedding_fn:
                text = f"{entity.entity_type}: {entity.name}"
                vector = self.embedding_fn(text)

            try:
                if vector:
                    self.collection.add_node(
                        id=entity_id,
                        label=entity.entity_type,
                        vector=vector,
                        metadata=metadata,
                    )
                else:
                    self.collection.add_node(
                        id=entity_id,
                        label=entity.entity_type,
                        metadata=metadata,
                    )
                nodes_added += 1
            except Exception:
                pass

        for relation in relations:
            source_id = entity_map.get(relation.source)
            target_id = entity_map.get(relation.target)

            if source_id is None or target_id is None:
                continue

            edge_id = _generate_id(
                f"{relation.source}->{relation.target}",
                relation.relation_type,
            )

            try:
                self.collection.add_edge(
                    id=edge_id,
                    source=source_id,
                    target=target_id,
                    label=relation.relation_type,
                    metadata=relation.properties,
                )
                edges_added += 1
            except Exception:
                pass

        return {"nodes": nodes_added, "edges": edges_added}

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
