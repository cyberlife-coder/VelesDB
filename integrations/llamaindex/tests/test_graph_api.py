"""Tests for graph API methods on LlamaIndex VelesDBVectorStore."""

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
        # Force collection initialization
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


# --- add_edge tests ---


class TestAddEdge:
    """Tests for VelesDBVectorStore.add_edge."""

    def test_add_edge_delegates(self, mock_vectorstore):
        vs, mock_col = mock_vectorstore
        vs.add_edge(id=1, source=100, target=200, label="KNOWS", metadata={"since": 2020})
        mock_col.add_edge.assert_called_once_with(
            id=1, source=100, target=200,
            label="KNOWS", metadata={"since": 2020},
        )

    def test_add_edge_validates_ids(self, mock_vectorstore):
        vs, _ = mock_vectorstore
        with pytest.raises(SecurityError, match="non-negative"):
            vs.add_edge(id=-1, source=100, target=200, label="KNOWS")

    def test_add_edge_validates_label(self, mock_vectorstore):
        vs, _ = mock_vectorstore
        with pytest.raises(SecurityError, match="alphanumeric"):
            vs.add_edge(id=1, source=100, target=200, label='"; DROP TABLE')

    def test_add_edge_no_collection(self, uninit_vectorstore):
        with pytest.raises(ValueError, match="Collection not initialized"):
            uninit_vectorstore.add_edge(id=1, source=100, target=200, label="KNOWS")


# --- get_edges tests ---


class TestGetEdges:
    """Tests for VelesDBVectorStore.get_edges."""

    def test_get_edges_all(self, mock_vectorstore):
        vs, mock_col = mock_vectorstore
        mock_col.get_edges.return_value = [
            {"id": 1, "source": 100, "target": 200, "label": "KNOWS", "properties": {}},
        ]
        result = vs.get_edges()
        mock_col.get_edges.assert_called_once()
        assert isinstance(result, list)
        assert len(result) == 1
        assert result[0]["label"] == "KNOWS"

    def test_get_edges_by_label(self, mock_vectorstore):
        vs, mock_col = mock_vectorstore
        mock_col.get_edges_by_label.return_value = [
            {"id": 1, "source": 100, "target": 200, "label": "KNOWS", "properties": {}},
        ]
        result = vs.get_edges(label="KNOWS")
        mock_col.get_edges_by_label.assert_called_once_with("KNOWS")
        assert len(result) == 1

    def test_get_edges_validates_label(self, mock_vectorstore):
        vs, _ = mock_vectorstore
        with pytest.raises(SecurityError, match="alphanumeric"):
            vs.get_edges(label="bad label!")

    def test_get_edges_no_collection(self, uninit_vectorstore):
        with pytest.raises(ValueError, match="Collection not initialized"):
            uninit_vectorstore.get_edges()


# --- traverse_graph tests ---


class TestTraverseGraph:
    """Tests for VelesDBVectorStore.traverse_graph."""

    def test_traverse_graph_returns_node_with_score(self, mock_vectorstore):
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = [
            {"target_id": 200, "depth": 1, "payload": {"text": "Connected node", "node_id": "200"}},
            {"target_id": 300, "depth": 2, "payload": {"text": "Second hop", "node_id": "300"}},
        ]
        result = vs.traverse_graph(source=100, max_depth=2)
        mock_col.traverse.assert_called_once_with(
            source=100, max_depth=2, strategy="bfs", limit=100,
        )
        assert isinstance(result, list)
        assert len(result) == 2
        assert all(isinstance(ns, NodeWithScore) for ns in result)
        assert result[0].node.text == "Connected node"
        assert result[1].node.text == "Second hop"

    def test_traverse_graph_invalid_strategy(self, mock_vectorstore):
        vs, _ = mock_vectorstore
        with pytest.raises(ValueError, match="Invalid strategy"):
            vs.traverse_graph(source=100, strategy="invalid")

    def test_traverse_graph_validates_source(self, mock_vectorstore):
        vs, _ = mock_vectorstore
        with pytest.raises(SecurityError, match="non-negative"):
            vs.traverse_graph(source=-1)

    def test_traverse_graph_depth_based_scoring(self, mock_vectorstore):
        """Verify depth-based scoring: score = 1.0 - (depth / (max_depth + 1))."""
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = [
            {"target_id": 200, "depth": 0, "payload": {"text": "Root"}},
            {"target_id": 201, "depth": 1, "payload": {"text": "Hop 1"}},
            {"target_id": 202, "depth": 2, "payload": {"text": "Hop 2"}},
        ]
        result = vs.traverse_graph(source=100, max_depth=2)
        # depth=0: 1.0 - 0/3 = 1.0
        assert abs(result[0].score - 1.0) < 1e-9
        # depth=1: 1.0 - 1/3 ≈ 0.6667
        assert abs(result[1].score - (1.0 - 1.0 / 3.0)) < 1e-9
        # depth=2: 1.0 - 2/3 ≈ 0.3333
        assert abs(result[2].score - (1.0 - 2.0 / 3.0)) < 1e-9

    def test_traverse_graph_metadata_includes_depth(self, mock_vectorstore):
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = [
            {"target_id": 200, "depth": 1, "payload": {"text": "Node A"}},
        ]
        result = vs.traverse_graph(source=100)
        assert result[0].node.metadata["graph_depth"] == 1
        assert result[0].node.metadata["target_id"] == 200

    def test_traverse_graph_no_collection(self, uninit_vectorstore):
        with pytest.raises(ValueError, match="Collection not initialized"):
            uninit_vectorstore.traverse_graph(source=100)

    def test_traverse_graph_dfs_strategy(self, mock_vectorstore):
        vs, mock_col = mock_vectorstore
        mock_col.traverse.return_value = []
        vs.traverse_graph(source=100, strategy="dfs")
        mock_col.traverse.assert_called_once_with(
            source=100, max_depth=2, strategy="dfs", limit=100,
        )


# --- get_node_degree tests ---


class TestGetNodeDegree:
    """Tests for VelesDBVectorStore.get_node_degree."""

    def test_get_node_degree_returns_dict(self, mock_vectorstore):
        vs, mock_col = mock_vectorstore
        mock_col.get_node_degree.return_value = {
            "node_id": 100, "in_degree": 3, "out_degree": 5, "total_degree": 8,
        }
        result = vs.get_node_degree(100)
        mock_col.get_node_degree.assert_called_once_with(100)
        assert isinstance(result, dict)
        assert result["total_degree"] == 8
        assert result["in_degree"] == 3
        assert result["out_degree"] == 5

    def test_get_node_degree_validates_id(self, mock_vectorstore):
        vs, _ = mock_vectorstore
        with pytest.raises(SecurityError, match="non-negative"):
            vs.get_node_degree(-1)

    def test_get_node_degree_no_collection(self, uninit_vectorstore):
        with pytest.raises(ValueError, match="Collection not initialized"):
            uninit_vectorstore.get_node_degree(100)
