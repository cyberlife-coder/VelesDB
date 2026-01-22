"""
Tests for VelesDB Graph operations (EPIC-016/US-030).

Run with: pytest tests/test_graph.py -v
"""

import pytest

# Import will fail until the module is built with maturin
try:
    from velesdb import GraphStore
except ImportError:
    pytest.skip(
        "velesdb module not built yet - run 'maturin develop' first",
        allow_module_level=True,
    )


class TestGetEdgesByLabel:
    """Tests for get_edges_by_label (US-030 AC-3)."""

    def test_get_edges_by_label_single_match(self):
        """Test get_edges_by_label returns correct edges."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
        store.add_edge({"id": 2, "source": 100, "target": 300, "label": "WORKS_AT"})

        knows_edges = store.get_edges_by_label("KNOWS")
        assert len(knows_edges) == 1
        assert knows_edges[0]["label"] == "KNOWS"

    def test_get_edges_by_label_multiple_matches(self):
        """Test get_edges_by_label returns all matching edges."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
        store.add_edge({"id": 2, "source": 200, "target": 300, "label": "KNOWS"})
        store.add_edge({"id": 3, "source": 100, "target": 400, "label": "FOLLOWS"})

        knows_edges = store.get_edges_by_label("KNOWS")
        assert len(knows_edges) == 2
        for edge in knows_edges:
            assert edge["label"] == "KNOWS"

    def test_get_edges_by_label_no_match(self):
        """Test get_edges_by_label returns empty list for non-existent label."""
        store = GraphStore()
        store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})

        empty_edges = store.get_edges_by_label("NONEXISTENT")
        assert len(empty_edges) == 0

    def test_get_edges_by_label_empty_store(self):
        """Test get_edges_by_label on empty store."""
        store = GraphStore()
        edges = store.get_edges_by_label("KNOWS")
        assert len(edges) == 0

    def test_get_edges_by_label_returns_dict(self):
        """Test get_edges_by_label returns list of dicts with correct keys."""
        store = GraphStore()
        store.add_edge({
            "id": 1,
            "source": 100,
            "target": 200,
            "label": "KNOWS",
            "properties": {"since": "2020"}
        })

        edges = store.get_edges_by_label("KNOWS")
        assert len(edges) == 1
        edge = edges[0]
        assert "id" in edge
        assert "source" in edge
        assert "target" in edge
        assert "label" in edge
