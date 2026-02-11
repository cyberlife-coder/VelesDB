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
        "id": 1,
        "payload": {"text": "Alice knows Bob", "label": "Person", "age": 30},
        "score": 0.95,
    },
    {
        "id": 2,
        "payload": {"text": "Bob knows Carol", "label": "Person", "age": 25},
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

    def test_match_query_returns_documents(self, store):
        """Happy path: returns List[Document] from collection.match_query()."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        results = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert isinstance(results, list)
        assert len(results) == 2
        assert all(isinstance(doc, Document) for doc in results)
        store._collection.match_query.assert_called_once_with(
            "MATCH (a:Person)-[:KNOWS]->(b) RETURN b", None
        )

    def test_match_query_validates_query(self, store):
        """SecurityError on malicious input."""
        with pytest.raises(SecurityError):
            store.match_query("MATCH (a); DROP TABLE users --")

    def test_match_query_no_collection(self, store_no_collection):
        """ValueError when collection not initialized."""
        with pytest.raises(ValueError, match="Collection not initialized"):
            store_no_collection.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")

    def test_match_query_converts_payload(self, store):
        """Verify payload → Document conversion (text + metadata)."""
        store._collection.match_query.return_value = MOCK_MATCH_RESULTS
        results = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")

        # First document
        assert results[0].page_content == "Alice knows Bob"
        assert results[0].metadata["label"] == "Person"
        assert results[0].metadata["age"] == 30
        assert "text" not in results[0].metadata

        # Second document
        assert results[1].page_content == "Bob knows Carol"
        assert results[1].metadata["label"] == "Person"
        assert results[1].metadata["age"] == 25

    def test_match_query_with_params(self, store):
        """Params dict passed through correctly."""
        store._collection.match_query.return_value = []
        params = {"label": "Person"}
        store.match_query("MATCH (a:Person) RETURN a", params=params)
        store._collection.match_query.assert_called_once_with(
            "MATCH (a:Person) RETURN a", params
        )

    def test_match_query_empty_results(self, store):
        """Returns empty list when no results."""
        store._collection.match_query.return_value = []
        results = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert results == []

    def test_match_query_missing_text_field(self, store):
        """Document created with empty text when payload has no 'text' key."""
        store._collection.match_query.return_value = [
            {"id": 1, "payload": {"label": "Node"}, "score": 0.5}
        ]
        results = store.match_query("MATCH (a) RETURN a")
        assert len(results) == 1
        assert results[0].page_content == ""
        assert results[0].metadata == {"label": "Node"}
