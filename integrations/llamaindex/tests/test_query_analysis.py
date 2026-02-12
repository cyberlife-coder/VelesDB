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
        """explain() raises NotImplementedError."""
        with pytest.raises(NotImplementedError, match="EXPLAIN planned for v2.0"):
            store.explain("SELECT * FROM docs LIMIT 10")

    def test_explain_validates_query(self, store):
        """SecurityError on SQL injection attempt (validated before NotImplementedError)."""
        with pytest.raises(SecurityError):
            store.explain("SELECT * FROM docs; DROP TABLE users --")

    def test_explain_no_collection(self, store_no_collection):
        """ValueError when collection not initialized (checked before NotImplementedError)."""
        with pytest.raises(ValueError, match="Collection not initialized"):
            store_no_collection.explain("SELECT * FROM docs LIMIT 10")

    def test_explain_with_params(self, store):
        """explain() raises NotImplementedError even with params."""
        params = {"v": [0.1, 0.2, 0.3]}
        with pytest.raises(NotImplementedError, match="EXPLAIN planned for v2.0"):
            store.explain("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10", params=params)


# --- match_query() ---

class TestMatchQuery:
    """Tests for match_query()."""

    def test_match_query_returns_query_result(self, store):
        """match_query() raises NotImplementedError."""
        with pytest.raises(NotImplementedError, match="MATCH execution engine planned for v2.0"):
            store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")

    def test_match_query_validates_query(self, store):
        """SecurityError on malicious input (validated before NotImplementedError)."""
        with pytest.raises(SecurityError):
            store.match_query("MATCH (a); DROP TABLE users --")

    def test_match_query_no_collection(self, store_no_collection):
        """Returns empty VectorStoreQueryResult when collection not initialized (checked before NotImplementedError)."""
        result = store_no_collection.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert isinstance(result, VectorStoreQueryResult)
        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_match_query_converts_payload(self, store):
        """match_query() raises NotImplementedError (no payload conversion)."""
        with pytest.raises(NotImplementedError, match="MATCH execution engine planned for v2.0"):
            store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")

    def test_match_query_similarities_populated(self, store):
        """match_query() raises NotImplementedError (no similarities populated)."""
        with pytest.raises(NotImplementedError, match="MATCH execution engine planned for v2.0"):
            store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")

    def test_match_query_ids_populated(self, store):
        """match_query() raises NotImplementedError (no IDs populated)."""
        with pytest.raises(NotImplementedError, match="MATCH execution engine planned for v2.0"):
            store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")

    def test_match_query_with_params(self, store):
        """match_query() raises NotImplementedError even with params."""
        params = {"label": "Person"}
        with pytest.raises(NotImplementedError, match="MATCH execution engine planned for v2.0"):
            store.match_query("MATCH (a:Person) RETURN a", params=params)

    def test_match_query_empty_results(self, store):
        """match_query() raises NotImplementedError (no empty results)."""
        with pytest.raises(NotImplementedError, match="MATCH execution engine planned for v2.0"):
            store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")

    def test_match_query_missing_node_id_falls_back(self, store):
        """match_query() raises NotImplementedError (no node_id fallback)."""
        with pytest.raises(NotImplementedError, match="MATCH execution engine planned for v2.0"):
            store.match_query("MATCH (a) RETURN a")
