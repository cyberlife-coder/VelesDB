"""Tests for stream_traverse_graph() on LangChain VelesDBVectorStore."""

import types
from unittest.mock import MagicMock, patch

import pytest
from langchain_core.documents import Document

from velesdb_common import SecurityError


@pytest.fixture
def mock_vectorstore():
    """Create a VelesDBVectorStore with a mocked collection."""
    with patch("langchain_velesdb.vectorstore.velesdb") as mock_velesdb:
        mock_db = MagicMock()
        mock_collection = MagicMock()
        mock_db.get_collection.return_value = mock_collection
        mock_velesdb.Database.return_value = mock_db

        from langchain_velesdb import VelesDBVectorStore

        mock_embedding = MagicMock()
        mock_embedding.embed_documents.return_value = [[0.1] * 128]
        mock_embedding.embed_query.return_value = [0.1] * 128

        vs = VelesDBVectorStore(
            embedding=mock_embedding,
            path="./test_data",
            collection_name="test",
        )
        vs._collection = mock_collection
        yield vs, mock_collection


@pytest.fixture
def uninit_vectorstore():
    """Create a VelesDBVectorStore without initialized collection."""
    with patch("langchain_velesdb.vectorstore.velesdb"):
        from langchain_velesdb import VelesDBVectorStore

        mock_embedding = MagicMock()
        vs = VelesDBVectorStore(
            embedding=mock_embedding,
            path="./test_data",
            collection_name="test",
        )
        vs._collection = None
        yield vs


TRAVERSAL_DATA = [
    {"target_id": 200, "depth": 0, "payload": {"text": "Root node", "node_id": "200"}},
    {"target_id": 201, "depth": 1, "payload": {"text": "First hop", "node_id": "201"}},
    {"target_id": 202, "depth": 2, "payload": {"text": "Second hop", "node_id": "202"}},
]


class TestStreamTraverseGraph:
    """Tests for VelesDBVectorStore.stream_traverse_graph."""

    def test_stream_traverse_is_generator(self, mock_vectorstore):
        """Verify return is a generator (has __next__)."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = TRAVERSAL_DATA
        result = vs.stream_traverse_graph(source=100, max_depth=2)
        assert isinstance(result, types.GeneratorType)

    def test_stream_traverse_yields_documents(self, mock_vectorstore):
        """Collect all yielded items, verify List[Document]."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = TRAVERSAL_DATA
        docs = list(vs.stream_traverse_graph(source=100, max_depth=2))
        assert len(docs) == 3
        assert all(isinstance(doc, Document) for doc in docs)
        assert docs[0].page_content == "Root node"
        assert docs[1].page_content == "First hop"
        assert docs[2].page_content == "Second hop"

    def test_stream_traverse_metadata(self, mock_vectorstore):
        """Each doc has graph_depth and target_id in metadata."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = TRAVERSAL_DATA
        docs = list(vs.stream_traverse_graph(source=100, max_depth=2))
        for i, doc in enumerate(docs):
            assert "graph_depth" in doc.metadata
            assert "target_id" in doc.metadata
            assert doc.metadata["graph_depth"] == TRAVERSAL_DATA[i]["depth"]
            assert doc.metadata["target_id"] == TRAVERSAL_DATA[i]["target_id"]

    def test_stream_traverse_validates_source(self, mock_vectorstore):
        """SecurityError on negative node_id."""
        vs, _ = mock_vectorstore
        with pytest.raises(SecurityError, match="non-negative"):
            # Must consume the generator to trigger validation
            list(vs.stream_traverse_graph(source=-1))

    def test_stream_traverse_invalid_strategy(self, mock_vectorstore):
        """ValueError on invalid strategy."""
        vs, _ = mock_vectorstore
        with pytest.raises(ValueError, match="Invalid strategy"):
            list(vs.stream_traverse_graph(source=100, strategy="invalid"))

    def test_stream_traverse_no_collection(self, uninit_vectorstore):
        """ValueError when not initialized."""
        with pytest.raises(ValueError, match="Collection not initialized"):
            list(uninit_vectorstore.stream_traverse_graph(source=100))

    def test_stream_traverse_empty_results(self, mock_vectorstore):
        """Generator yields nothing when traversal returns empty."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = []
        docs = list(vs.stream_traverse_graph(source=100))
        assert docs == []

    def test_stream_traverse_lazy_evaluation(self, mock_vectorstore):
        """Verify generator is lazy — items yielded one at a time."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = TRAVERSAL_DATA
        gen = vs.stream_traverse_graph(source=100, max_depth=2)
        first = next(gen)
        assert isinstance(first, Document)
        assert first.page_content == "Root node"
        second = next(gen)
        assert second.page_content == "First hop"
