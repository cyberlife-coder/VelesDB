"""Tests for stream_traverse_graph() on LlamaIndex VelesDBVectorStore."""

import types
from unittest.mock import MagicMock, patch

import pytest
from llama_index.core.schema import NodeWithScore, TextNode

from velesdb_common import SecurityError


@pytest.fixture
def mock_vectorstore():
    """Create a VelesDBVectorStore with a mocked collection."""
    with patch("llamaindex_velesdb.vectorstore.velesdb") as mock_velesdb:
        mock_db = MagicMock()
        mock_collection = MagicMock()
        mock_db.get_collection.return_value = mock_collection
        mock_velesdb.Database.return_value = mock_db

        from llamaindex_velesdb import VelesDBVectorStore

        vs = VelesDBVectorStore(
            path="./test_data",
            collection_name="test",
        )
        vs._collection = mock_collection
        yield vs, mock_collection


@pytest.fixture
def uninit_vectorstore():
    """Create a VelesDBVectorStore without initialized collection."""
    with patch("llamaindex_velesdb.vectorstore.velesdb"):
        from llamaindex_velesdb import VelesDBVectorStore

        vs = VelesDBVectorStore(
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

    def test_stream_traverse_yields_node_with_score(self, mock_vectorstore):
        """Collect all yielded items, verify List[NodeWithScore]."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = TRAVERSAL_DATA
        items = list(vs.stream_traverse_graph(source=100, max_depth=2))
        assert len(items) == 3
        assert all(isinstance(ns, NodeWithScore) for ns in items)
        assert items[0].node.text == "Root node"
        assert items[1].node.text == "First hop"
        assert items[2].node.text == "Second hop"

    def test_stream_traverse_depth_scores_decrease(self, mock_vectorstore):
        """Verify depth-based scores decrease with depth."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = TRAVERSAL_DATA
        items = list(vs.stream_traverse_graph(source=100, max_depth=2))
        # depth=0: 1.0 - 0/3 = 1.0
        assert abs(items[0].score - 1.0) < 1e-9
        # depth=1: 1.0 - 1/3 ≈ 0.6667
        assert abs(items[1].score - (1.0 - 1.0 / 3.0)) < 1e-9
        # depth=2: 1.0 - 2/3 ≈ 0.3333
        assert abs(items[2].score - (1.0 - 2.0 / 3.0)) < 1e-9
        # Scores strictly decreasing
        assert items[0].score > items[1].score > items[2].score

    def test_stream_traverse_metadata(self, mock_vectorstore):
        """Each node has graph_depth and target_id in metadata."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = TRAVERSAL_DATA
        items = list(vs.stream_traverse_graph(source=100, max_depth=2))
        for i, ns in enumerate(items):
            assert "graph_depth" in ns.node.metadata
            assert "target_id" in ns.node.metadata
            assert ns.node.metadata["graph_depth"] == TRAVERSAL_DATA[i]["depth"]
            assert ns.node.metadata["target_id"] == TRAVERSAL_DATA[i]["target_id"]

    def test_stream_traverse_validates_source(self, mock_vectorstore):
        """SecurityError on negative node_id."""
        vs, _ = mock_vectorstore
        with pytest.raises(SecurityError, match="non-negative"):
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
        items = list(vs.stream_traverse_graph(source=100))
        assert items == []

    def test_stream_traverse_lazy_evaluation(self, mock_vectorstore):
        """Verify generator is lazy — items yielded one at a time."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = TRAVERSAL_DATA
        gen = vs.stream_traverse_graph(source=100, max_depth=2)
        first = next(gen)
        assert isinstance(first, NodeWithScore)
        assert first.node.text == "Root node"
        second = next(gen)
        assert second.node.text == "First hop"
