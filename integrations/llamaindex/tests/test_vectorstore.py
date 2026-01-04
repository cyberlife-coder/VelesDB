"""Tests for VelesDB LlamaIndex VectorStore."""

import tempfile
import shutil
from pathlib import Path

import pytest
from llama_index.core.schema import TextNode

from llamaindex_velesdb import VelesDBVectorStore


class TestVelesDBVectorStore:
    """Test suite for VelesDBVectorStore."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def vector_store(self, temp_dir):
        """Create a VelesDBVectorStore instance."""
        return VelesDBVectorStore(
            path=temp_dir,
            collection_name="test",
            metric="cosine",
        )

    def test_init(self, temp_dir):
        """Test VectorStore initialization."""
        store = VelesDBVectorStore(path=temp_dir)
        assert store.path == temp_dir
        assert store.collection_name == "llamaindex"
        assert store.metric == "cosine"
        assert store.stores_text is True

    def test_add_nodes(self, vector_store):
        """Test adding nodes to the store."""
        nodes = [
            TextNode(
                text="Hello world",
                id_="node1",
                embedding=[0.1] * 768,
                metadata={"category": "greeting"},
            ),
            TextNode(
                text="Goodbye world",
                id_="node2",
                embedding=[0.2] * 768,
                metadata={"category": "farewell"},
            ),
        ]

        ids = vector_store.add(nodes)

        assert len(ids) == 2
        assert "node1" in ids
        assert "node2" in ids

    def test_add_empty_nodes(self, vector_store):
        """Test adding empty list returns empty."""
        ids = vector_store.add([])
        assert ids == []

    def test_query(self, vector_store):
        """Test querying the store."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        # Add nodes first
        nodes = [
            TextNode(
                text="VelesDB is a vector database",
                id_="doc1",
                embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            ),
            TextNode(
                text="LlamaIndex is a RAG framework",
                id_="doc2",
                embedding=[0.4, 0.5, 0.6] + [0.0] * 765,
            ),
        ]
        vector_store.add(nodes)

        # Query
        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            similarity_top_k=2,
        )
        result = vector_store.query(query)

        assert len(result.nodes) <= 2
        assert len(result.similarities) == len(result.nodes)
        assert len(result.ids) == len(result.nodes)

    def test_query_empty_embedding(self, vector_store):
        """Test query with no embedding returns empty."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        query = VectorStoreQuery(query_embedding=None)
        result = vector_store.query(query)

        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_delete(self, vector_store):
        """Test deleting a node."""
        nodes = [
            TextNode(
                text="To be deleted",
                id_="delete_me",
                embedding=[0.1] * 768,
            ),
        ]
        vector_store.add(nodes)

        # Delete should not raise
        vector_store.delete("delete_me")

    def test_client_property(self, vector_store):
        """Test client property returns database."""
        client = vector_store.client
        assert client is not None


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
