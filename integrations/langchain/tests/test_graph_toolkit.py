"""Tests for VelesDB Graph Toolkit."""

import pytest
from unittest.mock import Mock, patch

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

    def test_entity_creation(self):
        entity = Entity(name="John", entity_type="PERSON")
        assert entity.name == "John"
        assert entity.entity_type == "PERSON"
        assert entity.properties == {}

    def test_entity_with_properties(self):
        entity = Entity(
            name="Acme Corp",
            entity_type="ORGANIZATION",
            properties={"industry": "tech"},
        )
        assert entity.properties["industry"] == "tech"

    def test_entity_to_dict(self):
        entity = Entity(name="Paris", entity_type="LOCATION")
        d = entity.to_dict()
        assert d["name"] == "Paris"
        assert d["type"] == "LOCATION"


class TestRelation:
    """Tests for Relation dataclass."""

    def test_relation_creation(self):
        relation = Relation(
            source="John",
            target="Acme Corp",
            relation_type="WORKS_AT",
        )
        assert relation.source == "John"
        assert relation.target == "Acme Corp"
        assert relation.relation_type == "WORKS_AT"

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


class TestGraphLoader:
    """Tests for GraphLoader."""

    def test_loader_creation(self):
        mock_db = Mock()
        loader = GraphLoader(mock_db)
        assert loader.db == mock_db
        assert loader.collection_name == "knowledge_graph"

    def test_load_entities_and_relations(self):
        mock_collection = Mock()
        mock_collection.add_node = Mock()
        mock_collection.add_edge = Mock()

        mock_db = Mock()
        mock_db.get_or_create_collection = Mock(return_value=mock_collection)

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
        assert mock_collection.add_node.call_count == 2
        assert mock_collection.add_edge.call_count == 1

    def test_load_skips_invalid_relations(self):
        mock_collection = Mock()
        mock_collection.add_node = Mock()
        mock_collection.add_edge = Mock()

        mock_db = Mock()
        mock_db.get_or_create_collection = Mock(return_value=mock_collection)

        loader = GraphLoader(mock_db)

        entities = [Entity("John", "PERSON")]
        relations = [Relation("John", "Unknown", "KNOWS")]

        result = loader.load(entities, relations, generate_embeddings=False)

        assert result["nodes"] == 1
        assert result["edges"] == 0


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
