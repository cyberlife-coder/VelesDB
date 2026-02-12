"""Tests for collection and index management methods on VelesDBVectorStore.

Tests cover: list_collections, delete_collection, create_index, list_indexes, delete_index.
All tests use mocks — no server dependency.

Run with: pytest tests/test_collection_index.py -v
"""

from unittest.mock import MagicMock, patch

import pytest

try:
    from langchain_velesdb import VelesDBVectorStore
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


# --- list_collections ---

class TestListCollections:
    """Tests for list_collections()."""

    def test_list_collections_returns_list(self, store):
        """Happy path: returns list of collection name strings from db."""
        store._db.list_collections.return_value = ["col1", "col2"]
        result = store.list_collections()
        assert isinstance(result, list)
        assert len(result) == 2
        assert result[0] == "col1"
        store._db.list_collections.assert_called_once()

    def test_list_collections_empty(self, store):
        """Returns empty list when no collections exist."""
        store._db.list_collections.return_value = []
        result = store.list_collections()
        assert result == []


# --- delete_collection ---

class TestDeleteCollection:
    """Tests for delete_collection()."""

    def test_delete_collection_valid(self, store):
        """Happy path: delegates to db with validated name."""
        store.delete_collection("my_collection")
        store._db.delete_collection.assert_called_once_with("my_collection")

    def test_delete_collection_resets_current(self, store):
        """If deleting current collection, _collection becomes None."""
        store.delete_collection("test_col")
        assert store._collection is None
        store._db.delete_collection.assert_called_once_with("test_col")

    def test_delete_collection_other_keeps_current(self, store):
        """Deleting a different collection does not reset _collection."""
        store.delete_collection("other_col")
        assert store._collection is not None

    def test_delete_collection_invalid_name(self, store):
        """SecurityError on invalid collection name."""
        with pytest.raises(SecurityError):
            store.delete_collection("DROP TABLE; --")


# --- create_index ---

class TestCreateIndex:
    """Tests for create_property_index()."""

    def test_create_index_delegates(self, store):
        """Happy path: delegates to collection.create_property_index() which returns None."""
        store._collection.create_property_index.return_value = None
        result = store.create_property_index(label="Doc", property_name="category")
        store._collection.create_property_index.assert_called_once_with(label="Doc", property="category")
        assert result is None

    def test_create_index_no_collection(self, store_no_collection):
        """ValueError when collection not initialized."""
        with pytest.raises(ValueError, match="Collection not initialized"):
            store_no_collection.create_property_index(label="Doc", property_name="category")

    def test_create_index_invalid_label(self, store):
        """SecurityError on injection attempt in label."""
        with pytest.raises(SecurityError):
            store.create_property_index(label="'; DROP TABLE --", property_name="category")

    def test_create_index_invalid_property(self, store):
        """SecurityError on injection attempt in property."""
        with pytest.raises(SecurityError):
            store.create_property_index(label="Doc", property_name="cat; DELETE")


# --- list_indexes ---

class TestListIndexes:
    """Tests for list_indexes()."""

    def test_list_indexes_delegates(self, store):
        """Happy path: returns list from collection."""
        store._collection.list_indexes.return_value = [
            {"label": "Doc", "property": "category"},
        ]
        result = store.list_indexes()
        assert isinstance(result, list)
        assert len(result) == 1
        store._collection.list_indexes.assert_called_once()

    def test_list_indexes_no_collection(self, store_no_collection):
        """ValueError when collection not initialized."""
        with pytest.raises(ValueError, match="Collection not initialized"):
            store_no_collection.list_indexes()


# --- delete_index ---

class TestDeleteIndex:
    """Tests for drop_index()."""

    def test_delete_index_delegates(self, store):
        """Happy path: delegates to collection.drop_index() and returns bool."""
        store._collection.drop_index.return_value = True
        result = store.drop_index(label="Doc", property_name="category")
        store._collection.drop_index.assert_called_once_with(label="Doc", property="category")
        assert result is True

    def test_delete_index_no_collection(self, store_no_collection):
        """ValueError when collection not initialized."""
        with pytest.raises(ValueError, match="Collection not initialized"):
            store_no_collection.drop_index(label="Doc", property_name="category")

    def test_delete_index_invalid_params(self, store):
        """SecurityError on bad label."""
        with pytest.raises(SecurityError):
            store.drop_index(label="bad name!", property_name="category")

    def test_delete_index_invalid_property(self, store):
        """SecurityError on bad property."""
        with pytest.raises(SecurityError):
            store.drop_index(label="Doc", property_name="bad prop!")


# --- match_query and explain delegation tests ---

class TestDelegationMethods:
    """Tests for methods that delegate to collection."""

    def test_match_query_delegates(self, store):
        """match_query() delegates to collection.match_query()."""
        store._collection.match_query.return_value = []
        result = store.match_query("MATCH (a:Person)-[:KNOWS]->(b) RETURN b")
        assert result == []
        store._collection.match_query.assert_called_once()

    def test_explain_delegates(self, store):
        """explain() delegates to collection.explain()."""
        store._collection.explain.return_value = {"query_type": "SELECT"}
        result = store.explain("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10")
        assert isinstance(result, dict)
        store._collection.explain.assert_called_once()
