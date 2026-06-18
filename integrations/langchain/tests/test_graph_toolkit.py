"""Tests for VelesDB Graph Toolkit."""

import pytest
from unittest.mock import Mock

from langchain_velesdb.graph_toolkit import (
    GraphExtractor,
    ExtractionResult,
    Entity,
    Relation,
    GraphLoader,
    SemanticChunker,
)


class TestEntity:
    """Tests for Entity dataclass."""

    def test_entity_creation_defaults_properties_to_empty_dict(self):
        e1 = Entity(name="John", entity_type="PERSON")
        e2 = Entity(name="Mary", entity_type="PERSON")
        assert e1.properties == {}
        e1.properties["x"] = 1
        assert e2.properties == {}  # default_factory yields a fresh dict per instance

    def test_entity_to_dict(self):
        entity = Entity(name="Paris", entity_type="LOCATION")
        d = entity.to_dict()
        assert d["name"] == "Paris"
        assert d["type"] == "LOCATION"


class TestRelation:
    """Tests for Relation dataclass."""

    def test_relation_to_dict(self):
        relation = Relation(
            source="A",
            target="B",
            relation_type="RELATED_TO",
        )
        d = relation.to_dict()
        assert d["source"] == "A"
        assert d["target"] == "B"
        assert d["type"] == "RELATED_TO"


class TestExtractionResult:
    """Tests for ExtractionResult."""

    def test_empty_result(self):
        result = ExtractionResult()
        assert result.entities == []
        assert result.relations == []
        assert result.nodes == []
        assert result.edges == []

    def test_result_with_data(self):
        entities = [Entity("John", "PERSON")]
        relations = [Relation("John", "Acme", "WORKS_AT")]
        result = ExtractionResult(entities=entities, relations=relations)
        assert len(result.nodes) == 1
        assert len(result.edges) == 1


class TestGraphExtractor:
    """Tests for GraphExtractor."""

    def test_extract_empty_text(self):
        extractor = GraphExtractor()
        result = extractor.extract("")
        assert result.entities == []
        assert result.relations == []

    def test_extract_with_mock_llm(self):
        mock_client = Mock()
        mock_client.generate.return_value = '''
        {
            "entities": [
                {"name": "John", "type": "PERSON"},
                {"name": "Acme Corp", "type": "ORGANIZATION"}
            ],
            "relations": [
                {"source": "John", "target": "Acme Corp", "type": "WORKS_AT"}
            ]
        }
        '''

        extractor = GraphExtractor(llm_client=mock_client)
        result = extractor.extract("John works at Acme Corp.")

        assert len(result.entities) == 2
        assert len(result.relations) == 1
        assert result.entities[0].name == "John"
        assert result.relations[0].relation_type == "WORKS_AT"

    def test_parse_invalid_json(self):
        mock_client = Mock()
        mock_client.generate.return_value = "Invalid response"

        extractor = GraphExtractor(llm_client=mock_client)
        result = extractor.extract("Some text")

        assert result.entities == []
        assert result.relations == []

    def test_extract_batch(self):
        mock_client = Mock()
        mock_client.generate.return_value = '{"entities": [], "relations": []}'

        extractor = GraphExtractor(llm_client=mock_client)
        results = extractor.extract_batch(["Text 1", "Text 2"])

        assert len(results) == 2


class TestSemanticChunker:
    """Tests for SemanticChunker."""

    def test_chunk_short_text(self):
        chunker = SemanticChunker(chunk_size=1000)
        chunks = chunker.chunk("Short text")
        assert len(chunks) == 1
        assert chunks[0].text == "Short text"

    def test_chunk_empty_text(self):
        chunker = SemanticChunker()
        chunks = chunker.chunk("")
        assert chunks == []

    def test_chunk_long_text(self):
        chunker = SemanticChunker(chunk_size=100, chunk_overlap=20)
        long_text = "This is a sentence. " * 20
        chunks = chunker.chunk(long_text)
        assert len(chunks) > 1

    def test_chunk_with_entities(self):
        chunker = SemanticChunker(chunk_size=500)
        text = "John works at Acme Corp. Mary knows John."
        chunks = chunker.chunk(text, entities=["John", "Acme Corp", "Mary"])
        assert len(chunks) >= 1
        assert "John" in chunks[0].entities

    def test_chunk_preserves_positions(self):
        chunker = SemanticChunker(chunk_size=50, chunk_overlap=0)
        text = "First sentence. Second sentence. Third sentence."
        chunks = chunker.chunk(text)
        assert chunks[0].start_idx == 0

    def test_chunk_overlap_exceeds_chunk_length_no_negative_indices(self):
        """Test that large chunk_overlap doesn't produce negative start_idx.

        Bug: When chunk_overlap > len(current_chunk), the offset calculation
        (len(chunk) - overlap) became negative, causing negative start_idx.
        Fix: Clamp effective_overlap to min(chunk_overlap, len(current_chunk)).
        """
        chunker = SemanticChunker(chunk_size=50, chunk_overlap=200)
        text = "A. B. C. D. E. F. G. H. I. J. K. L. M. N. O. P. Q. R. S. T."
        chunks = chunker.chunk(text)

        for i, chunk in enumerate(chunks):
            assert chunk.start_idx >= 0, (
                f"Chunk {i} has negative start_idx: {chunk.start_idx}"
            )
            assert chunk.end_idx >= chunk.start_idx, (
                f"Chunk {i} has end_idx < start_idx: {chunk.end_idx} < {chunk.start_idx}"
            )
            assert chunk.end_idx <= len(text) + 200, (
                f"Chunk {i} has end_idx beyond text length: {chunk.end_idx}"
            )


class TestGraphLoader:
    """Tests for GraphLoader."""

    def test_loader_creation(self):
        mock_db = Mock()
        # default collection name is a public contract
        loader = GraphLoader(mock_db)
        assert loader.db is mock_db
        assert loader.collection_name == "knowledge_graph"
        # explicit collection_name must override the default (proves the param is wired)
        custom = GraphLoader(mock_db, collection_name="custom_graph")
        assert custom.collection_name == "custom_graph"

    def test_load_entities_and_relations(self):
        mock_collection = Mock()
        mock_collection.upsert = Mock()
        mock_collection.upsert_metadata = Mock()
        mock_collection.info = Mock(return_value={"metadata_only": True})

        mock_db = Mock()
        mock_db.get_collection = Mock(return_value=None)
        mock_db.create_metadata_collection = Mock(return_value=mock_collection)
        mock_graph_store = Mock()
        mock_db.graph_store = mock_graph_store

        loader = GraphLoader(mock_db)

        entities = [
            Entity("John", "PERSON"),
            Entity("Acme", "ORGANIZATION"),
        ]
        relations = [
            Relation("John", "Acme", "WORKS_AT"),
        ]

        result = loader.load(entities, relations, generate_embeddings=False)

        assert result["nodes"] == 2
        assert result["edges"] == 1
        assert mock_collection.upsert_metadata.call_count == 2
        assert mock_graph_store.add_edge.call_count == 1

    def test_load_skips_invalid_relations(self):
        mock_collection = Mock()
        mock_collection.upsert = Mock()
        mock_collection.upsert_metadata = Mock()
        mock_collection.info = Mock(return_value={"metadata_only": True})

        mock_db = Mock()
        mock_db.get_collection = Mock(return_value=None)
        mock_db.create_metadata_collection = Mock(return_value=mock_collection)
        mock_graph_store = Mock()
        mock_db.graph_store = mock_graph_store

        loader = GraphLoader(mock_db)

        entities = [Entity("John", "PERSON")]
        relations = [Relation("John", "Unknown", "KNOWS")]

        result = loader.load(entities, relations, generate_embeddings=False)

        assert result["nodes"] == 1
        assert result["edges"] == 0
        assert mock_graph_store.add_edge.call_count == 0

    def test_load_relation_uses_properties_key_not_metadata(self):
        """Contract parity: GraphStore edges must use `properties` key."""
        mock_collection = Mock()
        mock_collection.upsert = Mock()
        mock_collection.upsert_metadata = Mock()
        mock_collection.info = Mock(return_value={"metadata_only": True})

        mock_db = Mock()
        mock_db.get_collection = Mock(return_value=None)
        mock_db.create_metadata_collection = Mock(return_value=mock_collection)
        mock_graph_store = Mock()
        mock_db.graph_store = mock_graph_store

        loader = GraphLoader(mock_db)
        entities = [Entity("Alice", "PERSON"), Entity("Acme", "ORGANIZATION")]
        relations = [Relation("Alice", "Acme", "WORKS_AT", {"since": "2024"})]

        loader.load(entities, relations, generate_embeddings=False)

        call_arg = mock_graph_store.add_edge.call_args[0][0]
        assert "properties" in call_arg
        assert "metadata" not in call_arg
        assert call_arg["properties"] == {"since": "2024"}


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
