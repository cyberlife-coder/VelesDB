"""Tests for VelesDB LlamaIndex GraphLoader.

US-044: Knowledge Graph → LlamaIndex
"""

import pytest
from unittest.mock import MagicMock, patch


class TestGraphLoader:
    """Tests for GraphLoader class."""

    def test_init(self):
        """Test GraphLoader initialization."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        assert loader._vector_store is mock_store

    def test_add_node_with_metadata(self):
        """Test adding a node with metadata."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        loader.add_node(id=1, label="PERSON", metadata={"name": "John"})

        mock_collection.upsert_metadata.assert_called_once_with([
            {
                "id": 1,
                "payload": {"label": "PERSON", "name": "John"},
            }
        ])

    def test_add_node_with_vector(self):
        """Test adding a node with embedding vector."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        vector = [0.1, 0.2, 0.3]
        loader.add_node(id=1, label="DOCUMENT", vector=vector)

        mock_collection.upsert.assert_called_once()
        call_args = mock_collection.upsert.call_args[0][0][0]
        assert call_args["id"] == 1
        assert call_args["vector"] == vector
        assert call_args["payload"]["label"] == "DOCUMENT"

    def test_add_edge(self):
        """Test adding an edge."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        loader.add_edge(id=1, source=100, target=200, label="KNOWS")

        mock_collection.add_edge.assert_called_once_with(
            id=1,
            source=100,
            target=200,
            label="KNOWS",
            metadata={}
        )

    def test_add_edge_with_metadata(self):
        """Test adding an edge with properties."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        loader.add_edge(
            id=1,
            source=100,
            target=200,
            label="WORKS_AT",
            metadata={"since": "2024-01-01"}
        )

        mock_collection.add_edge.assert_called_once_with(
            id=1,
            source=100,
            target=200,
            label="WORKS_AT",
            metadata={"since": "2024-01-01"}
        )

    def test_get_edges_by_label(self):
        """Test getting edges filtered by label."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.get_edges_by_label.return_value = [
            {"id": 1, "source": 100, "target": 200, "label": "KNOWS", "properties": {}}
        ]
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        edges = loader.get_edges(label="KNOWS")

        mock_collection.get_edges_by_label.assert_called_once_with("KNOWS")
        assert len(edges) == 1
        assert edges[0]["label"] == "KNOWS"

    def test_get_edges_all(self):
        """Test getting all edges without filter."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.get_edges.return_value = [
            {"id": 1, "source": 100, "target": 200, "label": "KNOWS", "properties": {}},
            {"id": 2, "source": 200, "target": 300, "label": "FOLLOWS", "properties": {}}
        ]
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        edges = loader.get_edges()

        mock_collection.get_edges.assert_called_once()
        assert len(edges) == 2

    def test_get_edges_empty_collection(self):
        """Test getting edges from uninitialized collection."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None

        loader = GraphLoader(mock_store)
        edges = loader.get_edges()

        assert edges == []

    def test_load_from_nodes(self):
        """Test loading LlamaIndex nodes as graph nodes."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_store._collection = mock_collection

        # Mock LlamaIndex node
        mock_node = MagicMock()
        mock_node.node_id = "test-node-1"
        mock_node.get_content.return_value = "Test content"
        mock_node.metadata = {"source": "test.txt"}

        loader = GraphLoader(mock_store)
        result = loader.load_from_nodes([mock_node], node_label="DOCUMENT")

        assert result["nodes"] == 1
        assert result["edges"] == 0
        mock_collection.upsert_metadata.assert_called_once()

    def test_load_from_nodes_with_none_content(self):
        """Test loading nodes when get_content() returns None.
        
        Regression test: TypeError when node.get_content() returns None.
        Some LlamaIndex nodes may have no content (e.g., ImageNode without text).
        """
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_store._collection = mock_collection

        # Mock LlamaIndex node with None content
        mock_node = MagicMock()
        mock_node.node_id = "test-node-none-content"
        mock_node.get_content.return_value = None  # This caused TypeError
        mock_node.metadata = {"source": "image.png"}

        loader = GraphLoader(mock_store)
        # Should NOT raise TypeError: 'NoneType' object is not subscriptable
        result = loader.load_from_nodes([mock_node], node_label="IMAGE")

        assert result["nodes"] == 1
        assert result["edges"] == 0
        mock_collection.upsert_metadata.assert_called_once()
        
        # Verify text_preview is empty string, not None slice
        call_args = mock_collection.upsert_metadata.call_args
        actual_payload = call_args[0][0][0]["payload"]
        assert actual_payload["text_preview"] == ""

    def test_add_node_no_collection_raises(self):
        """Test that add_node raises when collection not initialized."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None

        loader = GraphLoader(mock_store)

        with pytest.raises(ValueError, match="Collection not initialized"):
            loader.add_node(id=1, label="TEST")

    def test_add_edge_no_collection_raises(self):
        """Test that add_edge raises when collection not initialized."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None

        loader = GraphLoader(mock_store)

        with pytest.raises(ValueError, match="Collection not initialized"):
            loader.add_edge(id=1, source=1, target=2, label="TEST")


class TestGraphLoaderIntegration:
    """Integration-style tests for GraphLoader.
    
    These tests verify the full flow without real VelesDB.
    """

    def test_full_graph_construction_flow(self):
        """Test complete graph construction workflow."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.get_edges_by_label.return_value = [
            {"id": 1, "source": 100, "target": 200, "label": "KNOWS", "properties": {}}
        ]
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)

        # Add nodes
        loader.add_node(id=100, label="PERSON", metadata={"name": "Alice"})
        loader.add_node(id=200, label="PERSON", metadata={"name": "Bob"})

        # Add edge
        loader.add_edge(id=1, source=100, target=200, label="KNOWS")

        # Query
        edges = loader.get_edges(label="KNOWS")

        assert len(edges) == 1
        assert edges[0]["source"] == 100
        assert edges[0]["target"] == 200
