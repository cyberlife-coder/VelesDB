"""
Tests for VelesDB Graph operations (EPIC-016/US-030, US-032).

Run with: pytest tests/test_graph.py -v
"""

import pytest

# Import will fail until the module is built with maturin
try:
    import velesdb
    from velesdb import GraphStore, StreamingConfig, TraversalResult
except ImportError:
    pytest.skip(
        "velesdb module not built yet - run 'maturin develop' first",
        allow_module_level=True,
    )


class TestGraphStore:
    """Tests for GraphStore class (US-030)."""

    def test_create_graph_store(self):
        """Test GraphStore creation."""
        store = GraphStore()
        assert store is not None
        assert store.edge_count() == 0

    def test_add_edge(self):
        """Test adding an edge."""
        store = GraphStore()
        store.add_edge(
            {"id": 1, "source": 100, "target": 200, "label": "KNOWS"}
        )
        assert store.edge_count() == 1

    def test_add_edge_with_properties(self):
        """Test adding an edge with properties."""
        store = GraphStore()
        store.add_edge(
            {
                "id": 1,
                "source": 100,
                "target": 200,
                "label": "KNOWS",
                "properties": {"since": "2020-01-01", "strength": 0.9},
            }
        )
        assert store.edge_count() == 1

    def test_get_edges_by_label(self):
        """Test get_edges_by_label (AC-3 from US-030)."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
        store.add_edge({"id": 2, "source": 100, "target": 300, "label": "WORKS_AT"})

        knows_edges = store.get_edges_by_label("KNOWS")
        assert len(knows_edges) == 1
        assert knows_edges[0]["label"] == "KNOWS"

        works_edges = store.get_edges_by_label("WORKS_AT")
        assert len(works_edges) == 1
        assert works_edges[0]["label"] == "WORKS_AT"

        # Non-existent label should return empty list
        empty_edges = store.get_edges_by_label("NONEXISTENT")
        assert len(empty_edges) == 0

    def test_get_outgoing(self):
        """Test getting outgoing edges from a node."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
        store.add_edge({"id": 2, "source": 100, "target": 300, "label": "FOLLOWS"})
        store.add_edge({"id": 3, "source": 200, "target": 300, "label": "KNOWS"})

        outgoing_100 = store.get_outgoing(100)
        assert len(outgoing_100) == 2

        outgoing_200 = store.get_outgoing(200)
        assert len(outgoing_200) == 1

    def test_get_incoming(self):
        """Test getting incoming edges to a node."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
        store.add_edge({"id": 2, "source": 300, "target": 200, "label": "FOLLOWS"})

        incoming_200 = store.get_incoming(200)
        assert len(incoming_200) == 2

    def test_get_outgoing_by_label(self):
        """Test getting outgoing edges filtered by label."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
        store.add_edge({"id": 2, "source": 100, "target": 300, "label": "FOLLOWS"})
        store.add_edge({"id": 3, "source": 100, "target": 400, "label": "KNOWS"})

        knows_from_100 = store.get_outgoing_by_label(100, "KNOWS")
        assert len(knows_from_100) == 2
        for edge in knows_from_100:
            assert edge["label"] == "KNOWS"

    def test_remove_edge(self):
        """Test removing an edge."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
        assert store.edge_count() == 1

        store.remove_edge(1)
        assert store.edge_count() == 0



class TestStreamingConfig:
    """Tests for StreamingConfig class."""

    def test_default_config(self):
        """Test default StreamingConfig values."""
        config = StreamingConfig()
        assert config.max_depth == 3
        assert config.max_visited == 10000
        assert config.relationship_types is None

    def test_custom_config(self):
        """Test custom StreamingConfig values."""
        config = StreamingConfig(max_depth=5, max_visited=500)
        assert config.max_depth == 5
        assert config.max_visited == 500

    def test_relationship_filter(self):
        """Test StreamingConfig with relationship type filter."""
        config = StreamingConfig(
            max_depth=2,
            max_visited=100,
            relationship_types=["KNOWS", "FOLLOWS"],
        )
        assert config.relationship_types == ["KNOWS", "FOLLOWS"]


class TestBfsStreaming:
    """Tests for BFS streaming traversal (US-032)."""

    def test_bfs_streaming_memory_bounded(self):
        """Test BFS streaming is memory bounded (AC-3 from US-032)."""
        store = GraphStore()

        # Create a graph with many edges
        for i in range(200):
            store.add_edge(
                {"id": i, "source": i, "target": i + 1, "label": "NEXT"}
            )

        config = StreamingConfig(max_depth=100, max_visited=50)
        results = store.traverse_bfs_streaming(0, config)

        # Results should be bounded by max_visited
        assert len(results) <= 50

    def test_bfs_streaming_depth_limited(self):
        """Test BFS streaming respects max_depth."""
        store = GraphStore()

        # Linear chain: 0 -> 1 -> 2 -> 3 -> 4
        for i in range(5):
            store.add_edge(
                {"id": i, "source": i, "target": i + 1, "label": "NEXT"}
            )

        config = StreamingConfig(max_depth=2, max_visited=1000)
        results = store.traverse_bfs_streaming(0, config)

        # Should only reach depth 2
        for result in results:
            assert result.depth <= 2

    def test_bfs_streaming_relationship_filter(self):
        """Test BFS streaming with relationship type filter."""
        store = GraphStore()

        store.add_edge({"id": 1, "source": 0, "target": 1, "label": "KNOWS"})
        store.add_edge({"id": 2, "source": 0, "target": 2, "label": "BLOCKED"})
        store.add_edge({"id": 3, "source": 1, "target": 3, "label": "KNOWS"})

        config = StreamingConfig(
            max_depth=3,
            max_visited=100,
            relationship_types=["KNOWS"],
        )
        results = store.traverse_bfs_streaming(0, config)

        # Should only traverse KNOWS edges
        for result in results:
            assert result.label == "KNOWS"

        # Should have traversed 0->1 and 1->3
        assert len(results) == 2

    def test_bfs_streaming_result_fields(self):
        """Test TraversalResult fields are correct."""
        store = GraphStore()
        store.add_edge({"id": 42, "source": 100, "target": 200, "label": "KNOWS"})

        config = StreamingConfig(max_depth=1, max_visited=10)
        results = store.traverse_bfs_streaming(100, config)

        assert len(results) == 1
        result = results[0]
        assert result.depth == 1
        assert result.source == 100
        assert result.target == 200
        assert result.label == "KNOWS"
        assert result.edge_id == 42

    def test_bfs_streaming_empty_graph(self):
        """Test BFS streaming on empty graph."""
        store = GraphStore()
        config = StreamingConfig(max_depth=3, max_visited=100)
        results = store.traverse_bfs_streaming(0, config)
        assert len(results) == 0

    def test_bfs_streaming_isolated_node(self):
        """Test BFS streaming from isolated node."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})

        config = StreamingConfig(max_depth=3, max_visited=100)
        # Start from node 999 which has no edges
        results = store.traverse_bfs_streaming(999, config)
        assert len(results) == 0
