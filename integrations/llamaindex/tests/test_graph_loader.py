"""Tests for VelesDB LlamaIndex GraphLoader.

US-044: Knowledge Graph → LlamaIndex
"""

import pytest
from unittest.mock import MagicMock


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
        mock_collection.info.return_value = {"metadata_only": True}
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        loader.add_node(id=1, label="PERSON", metadata={"name": "John"})

        mock_collection.upsert_metadata.assert_called_once_with(
            [{"id": 1, "payload": {"label": "PERSON", "name": "John"}}]
        )

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

        loader = GraphLoader(mock_store)
        loader.add_edge(id=1, source=100, target=200, label="KNOWS")

        edges = loader.get_edges()
        assert len(edges) == 1
        assert edges[0]["label"] == "KNOWS"

    def test_add_edge_with_metadata(self):
        """Test adding an edge with properties."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()

        loader = GraphLoader(mock_store)
        loader.add_edge(
            id=1,
            source=100,
            target=200,
            label="WORKS_AT",
            metadata={"since": "2024-01-01"}
        )

        edges = loader.get_edges(label="WORKS_AT")
        assert len(edges) == 1
        assert edges[0]["properties"] == {"since": "2024-01-01"}

    def test_get_edges_by_label(self):
        """Test getting edges filtered by label."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        loader.add_edge(id=1, source=100, target=200, label="KNOWS")
        edges = loader.get_edges(label="KNOWS")

        assert len(edges) == 1
        assert edges[0]["label"] == "KNOWS"

    def test_get_edges_all(self):
        """Test getting all edges without filter."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        loader.add_edge(id=1, source=100, target=200, label="KNOWS")
        loader.add_edge(id=2, source=200, target=300, label="FOLLOWS")
        edges = loader.get_edges()

        assert len(edges) == 2

    def test_get_edges_empty_collection(self):
        """Test getting edges from uninitialized collection."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        edges = loader.get_edges()

        assert edges == []

    def test_load_from_nodes(self):
        """Test loading LlamaIndex nodes as graph nodes."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": True}
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
        mock_collection.info.return_value = {"metadata_only": True}
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
        call_args = mock_collection.upsert_metadata.call_args[0][0][0]
        assert call_args["payload"]["text_preview"] == ""

    def test_load_from_nodes_on_vector_collection_without_vectors(self):
        """load_from_nodes should still store metadata on vector collections."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": False}
        mock_store._collection = mock_collection

        mock_node = MagicMock()
        mock_node.node_id = "test-node-vector-collection"
        mock_node.get_content.return_value = "Vector collection node"
        mock_node.metadata = {"source": "doc.txt"}

        loader = GraphLoader(mock_store)
        result = loader.load_from_nodes([mock_node], node_label="DOCUMENT")

        assert result["nodes"] == 1
        assert result["edges"] == 0
        mock_collection.upsert_metadata.assert_called_once()

    def test_add_node_no_collection_raises(self):
        """Test that add_node raises when collection cannot be initialized."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None
        mock_store._get_collection = None

        loader = GraphLoader(mock_store)

        with pytest.raises(ValueError, match="cannot initialize collection"):
            loader.add_node(id=1, label="TEST")

    def test_add_edge_no_collection_raises(self):
        """Test that add_edge still works without collection initialization."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None

        loader = GraphLoader(mock_store)
        loader.add_edge(id=1, source=1, target=2, label="TEST")
        assert len(loader.get_edges()) == 1


class TestGraphLoaderIntegration:
    """Integration-style tests for GraphLoader.
    
    These tests verify the full flow without real VelesDB.
    """

    def test_full_graph_construction_flow(self):
        """Test complete graph construction workflow."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": True}
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

    def test_get_collection_initializes_from_vector_store(self):
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": True}
        mock_store._get_collection = MagicMock(return_value=mock_collection)

        loader = GraphLoader(mock_store)
        loader.add_node(id=1, label="PERSON", metadata={"name": "A"})

        mock_store._get_collection.assert_called_once()


class TestGraphLoaderNativeGraph:
    """Tests for GraphLoader.get_edges when a native graph collection is present.

    Issue #5 (PR-306): get_edges only read self._edges; edges persisted in
    the native graph were invisible across sessions.
    """

    def _make_loader_with_native_graph(self, native_graph_mock):
        """Return a GraphLoader whose _native_graph is pre-set to *native_graph_mock*."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        # Bypass _open_native_graph by injecting the mock directly.
        loader._native_graph = native_graph_mock
        return loader

    def test_get_edges_reads_from_native_graph_when_available(self):
        """get_edges delegates to native graph and normalises the result."""
        native = MagicMock()
        native.get_edges.return_value = [
            {"id": 7, "source": 1, "target": 2, "label": "KNOWS", "properties": {}},
        ]

        loader = self._make_loader_with_native_graph(native)
        edges = loader.get_edges()

        native.get_edges.assert_called_once_with(None)
        assert len(edges) == 1
        assert edges[0] == {
            "id": 7,
            "source": 1,
            "target": 2,
            "label": "KNOWS",
            "properties": {},
        }

    def test_get_edges_passes_label_filter_to_native_graph(self):
        """get_edges forwards the label argument to native graph."""
        native = MagicMock()
        native.get_edges.return_value = [
            {"id": 3, "source": 10, "target": 20, "label": "WORKS_AT", "properties": {"since": "2024"}},
        ]

        loader = self._make_loader_with_native_graph(native)
        edges = loader.get_edges(label="WORKS_AT")

        native.get_edges.assert_called_once_with("WORKS_AT")
        assert len(edges) == 1
        assert edges[0]["label"] == "WORKS_AT"
        assert edges[0]["properties"] == {"since": "2024"}

    def test_get_edges_normalises_missing_fields_from_native_graph(self):
        """Native graph entries with missing fields are filled with safe defaults."""
        native = MagicMock()
        # Return a partial edge — some implementations may omit optional keys.
        native.get_edges.return_value = [{"label": "REL"}]

        loader = self._make_loader_with_native_graph(native)
        edges = loader.get_edges()

        assert edges[0]["id"] == 0
        assert edges[0]["source"] == 0
        assert edges[0]["target"] == 0
        assert edges[0]["properties"] == {}

    def test_get_edges_ignores_in_memory_edges_when_native_graph_present(self):
        """In-memory self._edges are NOT returned when native graph is available."""
        native = MagicMock()
        native.get_edges.return_value = []

        loader = self._make_loader_with_native_graph(native)
        # Manually pollute the in-memory list — should NOT appear in results.
        loader._edges.append(
            {"id": 99, "source": 1, "target": 2, "label": "STALE", "properties": {}}
        )

        edges = loader.get_edges()

        assert edges == []

    def test_get_edges_falls_back_to_in_memory_when_native_graph_is_none(self):
        """Without a native graph the existing in-memory behaviour is preserved."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        # Confirm no native graph was injected.
        loader._native_graph = None

        loader.add_edge(id=5, source=1, target=2, label="KNOWS")
        edges = loader.get_edges(label="KNOWS")

        assert len(edges) == 1
        assert edges[0]["id"] == 5

    def test_get_edges_native_graph_exception_falls_back_to_in_memory(self):
        """If native graph raises, get_edges falls back to in-memory list."""
        native = MagicMock()
        native.get_edges.side_effect = RuntimeError("native graph offline")

        loader = self._make_loader_with_native_graph(native)
        loader._edges.append(
            {"id": 1, "source": 10, "target": 20, "label": "KNOWS", "properties": {}}
        )

        edges = loader.get_edges()

        # Fallback: the in-memory edge is returned.
        assert len(edges) == 1
        assert edges[0]["id"] == 1


class TestGraphRetriever:
    def test_fetch_node_uses_get_nodes(self):
        from llamaindex_velesdb import GraphRetriever
        from llama_index.core.schema import TextNode

        mock_vector_store = MagicMock()
        expected = TextNode(text="Neighbor", id_="42")
        mock_vector_store.get_nodes.return_value = [expected]

        mock_index = MagicMock()
        mock_index._vector_store = mock_vector_store

        retriever = GraphRetriever(index=mock_index)
        node = retriever._fetch_node(42)

        mock_vector_store.get_nodes.assert_called_once_with(["42"])
        assert node == expected
