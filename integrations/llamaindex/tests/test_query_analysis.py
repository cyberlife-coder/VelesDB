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
        "node_id": 1,
        "depth": 0,
        "path": [],
        "bindings": {"a": 1},
        "score": 0.95,
        "projected": {"name": "Alice"},
    },
    {
        "node_id": 2,
        "depth": 1,
        "path": [100],
        "bindings": {"b": 2},
        "score": 0.88,
        "projected": {"name": "Bob"},
    },
]


# --- explain() ---

class TestExplain:
    """Tests for explain()."""

    def test_explain_returns_dict(self, store):
        """explain() delegates to collection.explain() and returns dict."""
        store._collection.explain.return_value = MOCK_EXPLAIN_RESULT
        result = store.explain("SELECT * FROM docs LIMIT 10")
        assert isinstance(result, dict)
        store._collection.explain.assert_called_once_with("SELECT * FROM docs LIMIT 10")

    def test_explain_validates_query(self, store):
        """SecurityError on SQL injection attempt (validated before delegation)."""
        with pytest.raises(SecurityError):
            store.explain("SELECT * FROM docs; DROP TABLE users --")

    def test_explain_no_collection(self, store_no_collection):
        """ValueError when collection not initialized."""
        with pytest.raises(ValueError, match="Collection not initialized"):
            store_no_collection.explain("SELECT * FROM docs LIMIT 10")

    def test_explain_with_params(self, store):
        """explain() delegates even with params."""
        store._collection.explain.return_value = MOCK_EXPLAIN_RESULT
        params = {"v": [0.1, 0.2, 0.3]}
        result = store.explain("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10", params=params)
        assert isinstance(result, dict)
        store._collection.explain.assert_called_once()


# --- match_query() ---

class TestMatchQuery:
    """Tests for match_query()."""

    def test_match_query_returns_query_result(self, store):
        """match_query() delegates and returns VectorStoreQueryResult."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert isinstance(result, VectorStoreQueryResult)
        assert len(result.nodes) == 2
        assert len(result.similarities) == 2
        assert len(result.ids) == 2
        store._collection.match_query.assert_called_once()

    def test_match_query_validates_query(self, store):
        """SecurityError on malicious input (validated before delegation)."""
        with pytest.raises(SecurityError):
            store.match_query("MATCH (a); DROP TABLE users --")

    def test_match_query_no_collection(self, store_no_collection):
        """Returns empty VectorStoreQueryResult when collection not initialized."""
        result = store_no_collection.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert isinstance(result, VectorStoreQueryResult)
        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_match_query_nodes_have_text(self, store):
        """TextNode text is from projected or bindings."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert result.nodes[0].text != ""

    def test_match_query_similarities_populated(self, store):
        """Similarities come from result score field."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert result.similarities[0] == 0.95
        assert result.similarities[1] == 0.88

    def test_match_query_ids_populated(self, store):
        """IDs come from node_id field."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert result.ids[0] == "1"
        assert result.ids[1] == "2"

    def test_match_query_with_params(self, store):
        """match_query() delegates with params."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        params = {"label": "Person"}
        result = store.match_query("MATCH (a:Person) RETURN a", params=params)
        assert len(result.nodes) == 2
        store._collection.match_query.assert_called_once()

    def test_match_query_empty_results(self, store):
        """Empty list from SDK returns empty VectorStoreQueryResult."""
        store._collection.match_query.return_value = []
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []
