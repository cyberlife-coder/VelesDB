"""Tests for Collection.search_with_ef() and search_ids() bindings (Phase 4.3 Plan 02).

Validates that PyO3 bindings correctly delegate to core's
search_with_ef() and search_ids().

Run with: pytest tests/test_search_advanced.py -v
Requires: maturin develop
"""

import pytest
import tempfile
import shutil

try:
    import velesdb
except ImportError:
    pytest.skip(
        "velesdb module not built yet - run 'maturin develop' first",
        allow_module_level=True,
    )


@pytest.fixture
def collection_with_vectors():
    """Create a collection with vectors for search testing."""
    path = tempfile.mkdtemp(prefix="velesdb_search_adv_test_")
    db = velesdb.Database(path)
    col = db.create_collection("test_search", dimension=4, metric="cosine")

    col.upsert(
        [
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"name": "a"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"name": "b"}},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"name": "c"}},
            {"id": 4, "vector": [0.0, 0.0, 0.0, 1.0], "payload": {"name": "d"}},
        ]
    )

    yield col
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def empty_collection():
    """Create an empty collection."""
    path = tempfile.mkdtemp(prefix="velesdb_search_empty_test_")
    db = velesdb.Database(path)
    col = db.create_collection("test_empty", dimension=4, metric="cosine")
    yield col
    shutil.rmtree(path, ignore_errors=True)


def test_search_with_ef_basic(collection_with_vectors):
    """Search with custom ef_search, verify results."""
    results = collection_with_vectors.search_with_ef(
        vector=[1.0, 0.0, 0.0, 0.0],
        top_k=2,
        ef_search=200,
    )
    assert isinstance(results, list)
    assert len(results) >= 1
    # Verify dict keys
    for r in results:
        assert "id" in r
        assert "score" in r
        assert "payload" in r


def test_search_with_ef_dimension_mismatch(collection_with_vectors):
    """Wrong dimension raises error."""
    with pytest.raises(Exception):
        collection_with_vectors.search_with_ef(
            vector=[1.0, 0.0],  # dim=2, expected dim=4
            top_k=2,
            ef_search=100,
        )


def test_search_ids_basic(collection_with_vectors):
    """Search returning only (id, score) tuples."""
    results = collection_with_vectors.search_ids(
        vector=[1.0, 0.0, 0.0, 0.0],
        top_k=2,
    )
    assert isinstance(results, list)
    assert len(results) >= 1
    # Each result is a tuple (id, score)
    for r in results:
        assert isinstance(r, tuple)
        assert len(r) == 2
        assert isinstance(r[0], int)
        assert isinstance(r[1], float)


def test_search_ids_empty_collection(empty_collection):
    """Empty collection returns empty list."""
    results = empty_collection.search_ids(
        vector=[1.0, 0.0, 0.0, 0.0],
        top_k=10,
    )
    assert isinstance(results, list)
    assert len(results) == 0
