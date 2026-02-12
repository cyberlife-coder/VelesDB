"""Tests for explain() and match_query() on VelesDBVectorStore.

Tests cover query plan analysis and MATCH graph traversal.
All tests use mocks — no server dependency.

Run with: pytest tests/test_query_analysis.py -v
"""

from unittest.mock import MagicMock

import pytest

try:
    from langchain_velesdb import VelesDBVectorStore
    from langchain_core.documents import Document
    from langchain_core.embeddings import Embeddings
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)

from velesdb_common import SecurityError


class FakeEmbeddings(Embeddings):
    """Minimal fake embeddings for testing."""

    def embed_documents(self, texts):
        return [[0.1, 0.2, 0.3, 0.4] for _ in texts]

    def embed_query(self, text):
        return [0.1, 0.2, 0.3, 0.4]


@pytest.fixture
def store():
    """Create a VelesDBVectorStore with mocked internals."""
    s = VelesDBVectorStore(embedding=FakeEmbeddings(), path="./test_data", collection_name="test_col")
    s._db = MagicMock()
    s._collection = MagicMock()
    return s


@pytest.fixture
def store_no_collection():
    """Store without an initialized collection."""
    s = VelesDBVectorStore(embedding=FakeEmbeddings(), path="./test_data", collection_name="test_col")
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

    def test_match_query_returns_documents(self, store):
        """match_query() delegates and returns Document list."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        results = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert isinstance(results, list)
        assert len(results) == 2
        assert isinstance(results[0], Document)
        store._collection.match_query.assert_called_once()

    def test_match_query_validates_query(self, store):
        """SecurityError on malicious input (validated before delegation)."""
        with pytest.raises(SecurityError):
            store.match_query("MATCH (a); DROP TABLE users --")

    def test_match_query_no_collection(self, store_no_collection):
        """Empty list when collection not initialized."""
        results = store_no_collection.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert results == []

    def test_match_query_metadata_populated(self, store):
        """match_query() populates Document metadata from result dict."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        results = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        meta = results[0].metadata
        assert meta["node_id"] == 1
        assert meta["depth"] == 0
        assert "bindings" in meta

    def test_match_query_with_params(self, store):
        """match_query() delegates with params."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        params = {"label": "Person"}
        results = store.match_query("MATCH (a:Person) RETURN a", params=params)
        assert len(results) == 2
        store._collection.match_query.assert_called_once()

    def test_match_query_empty_results(self, store):
        """Empty list from SDK returns empty Document list."""
        store._collection.match_query.return_value = []
        results = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert results == []

    def test_match_query_page_content_from_projected(self, store):
        """page_content uses projected or bindings."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        results = store.match_query("MATCH (a) RETURN a")
        assert results[0].page_content != ""
