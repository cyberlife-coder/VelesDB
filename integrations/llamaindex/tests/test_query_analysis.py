"""Tests for explain() and match_query() on VelesDBVectorStore.

Tests cover query plan analysis and MATCH graph traversal.
All tests use mocks — no server dependency.

Run with: pytest tests/test_query_analysis.py -v
"""

from unittest.mock import MagicMock

import pytest

from llamaindex_velesdb import VelesDBVectorStore
from llama_index.core.schema import TextNode
from llama_index.core.vector_stores.types import VectorStoreQueryResult
from velesdb_common import SecurityError


@pytest.fixture
def store():
    """Create a VelesDBVectorStore with mocked internals."""
    s = VelesDBVectorStore(path="./test_data", collection_name="test_col")
    s._db = MagicMock()
    s._collection = MagicMock()
    return s


@pytest.fixture
def store_no_collection():
    """Store without an initialized collection."""
    s = VelesDBVectorStore(path="./test_data", collection_name="test_col")
    s._db = MagicMock()
    s._collection = None
    return s


MOCK_EXPLAIN_RESULT = {
    "steps": [{"type": "scan", "collection": "docs"}],
    "cost": {"estimated_rows": 100},
    "features": {"similarity": True},
}

MOCK_MATCH_RESULTS = [
    {
        "id": 1,
        "payload": {
            "text": "Alice knows Bob",
            "node_id": "1",
            "label": "Person",
            "age": 30,
        },
        "score": 0.95,
    },
    {
        "id": 2,
        "payload": {
            "text": "Bob knows Carol",
            "node_id": "2",
            "label": "Person",
            "age": 25,
        },
        "score": 0.88,
    },
]


# --- explain() ---

class TestExplain:
    """Tests for explain()."""

    def test_explain_returns_dict(self, store):
        """Happy path: returns raw dict from collection.explain()."""
        store._collection.explain.return_value = MOCK_EXPLAIN_RESULT
        result = store.explain("SELECT * FROM docs LIMIT 10")
        assert isinstance(result, dict)
        assert "steps" in result
        assert "cost" in result
        assert "features" in result
        store._collection.explain.assert_called_once_with(
            "SELECT * FROM docs LIMIT 10", None
        )

    def test_explain_validates_query(self, store):
        """SecurityError on SQL injection attempt."""
        with pytest.raises(SecurityError):
            store.explain("SELECT * FROM docs; DROP TABLE users --")

    def test_explain_no_collection(self, store_no_collection):
        """ValueError when collection not initialized."""
        with pytest.raises(ValueError, match="Collection not initialized"):
            store_no_collection.explain("SELECT * FROM docs LIMIT 10")

    def test_explain_with_params(self, store):
        """Params dict passed through correctly to collection."""
        store._collection.explain.return_value = MOCK_EXPLAIN_RESULT
        params = {"v": [0.1, 0.2, 0.3]}
        result = store.explain("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10", params=params)
        assert isinstance(result, dict)
        store._collection.explain.assert_called_once_with(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10", params
        )


# --- match_query() ---

class TestMatchQuery:
    """Tests for match_query()."""

    def test_match_query_returns_query_result(self, store):
        """Happy path: returns VectorStoreQueryResult with TextNode objects."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert isinstance(result, VectorStoreQueryResult)
        assert len(result.nodes) == 2
        assert all(isinstance(n, TextNode) for n in result.nodes)
        store._collection.match_query.assert_called_once_with(
            "MATCH (a:Person)-[:KNOWS]->(b) RETURN b", None
        )

    def test_match_query_validates_query(self, store):
        """SecurityError on malicious input."""
        with pytest.raises(SecurityError):
            store.match_query("MATCH (a); DROP TABLE users --")

    def test_match_query_no_collection(self, store_no_collection):
        """Returns empty VectorStoreQueryResult when collection not initialized."""
        result = store_no_collection.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert isinstance(result, VectorStoreQueryResult)
        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_match_query_converts_payload(self, store):
        """Verify payload → TextNode conversion (text + metadata + node_id)."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")

        # First node
        assert result.nodes[0].text == "Alice knows Bob"
        assert result.nodes[0].id_ == "1"
        assert result.nodes[0].metadata["label"] == "Person"
        assert result.nodes[0].metadata["age"] == 30
        assert "text" not in result.nodes[0].metadata
        assert "node_id" not in result.nodes[0].metadata

        # Second node
        assert result.nodes[1].text == "Bob knows Carol"
        assert result.nodes[1].id_ == "2"

    def test_match_query_similarities_populated(self, store):
        """Verify similarities list is populated from result scores."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert result.similarities == [0.95, 0.88]

    def test_match_query_ids_populated(self, store):
        """Verify IDs list is populated from node_id fields."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert result.ids == ["1", "2"]

    def test_match_query_with_params(self, store):
        """Params dict passed through correctly."""
        store._collection.match_query.return_value = []
        params = {"label": "Person"}
        store.match_query("MATCH (a:Person) RETURN a", params=params)
        store._collection.match_query.assert_called_once_with(
            "MATCH (a:Person) RETURN a", params
        )

    def test_match_query_empty_results(self, store):
        """Returns empty result when no matches."""
        store._collection.match_query.return_value = []
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_match_query_missing_node_id_falls_back(self, store):
        """Uses result id as fallback when payload has no 'node_id' key."""
        store._collection.match_query.return_value = [
            {"id": 42, "payload": {"text": "orphan node", "label": "Node"}, "score": 0.5}
        ]
        result = store.match_query("MATCH (a) RETURN a")
        assert len(result.nodes) == 1
        assert result.nodes[0].id_ == "42"
        assert result.ids == ["42"]
