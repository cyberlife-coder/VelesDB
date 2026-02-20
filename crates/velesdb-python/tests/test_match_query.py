"""Tests for MATCH query and EXPLAIN bindings."""

import shutil
import tempfile

import pytest

try:
    import velesdb
except ImportError:
    pytest.skip("velesdb module not built yet - run 'maturin develop' first", allow_module_level=True)


@pytest.fixture
def temp_db_path():
    path = tempfile.mkdtemp(prefix="velesdb_match_test_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


def _seed_collection(collection):
    collection.upsert(
        [
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"name": "A"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"name": "B"}},
            {"id": 3, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"name": "C"}},
        ]
    )


def test_match_query_basic(temp_db_path):
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("match_basic", dimension=4, metric="cosine")
    _seed_collection(collection)

    results = collection.match_query("MATCH (n) RETURN n LIMIT 2")
    assert len(results) <= 2
    if results:
        sample = results[0]
        assert "node_id" in sample
        assert "depth" in sample
        assert "path" in sample
        assert "bindings" in sample
        assert "score" in sample
        assert "projected" in sample


def test_match_query_with_similarity(temp_db_path):
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("match_sim", dimension=4, metric="cosine")
    _seed_collection(collection)

    results = collection.match_query(
        "MATCH (n) RETURN n LIMIT 10",
        vector=[1.0, 0.0, 0.0, 0.0],
        threshold=0.5,
    )
    assert results
    assert all(r.get("score") is not None for r in results)


def test_match_query_invalid_non_match_query(temp_db_path):
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("match_invalid", dimension=4, metric="cosine")
    _seed_collection(collection)

    with pytest.raises(Exception):
        collection.match_query("SELECT * FROM match_invalid LIMIT 1")


def test_explain_returns_plan_dict(temp_db_path):
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("explain_test", dimension=4, metric="cosine")
    _seed_collection(collection)

    explain = collection.explain("SELECT * FROM explain_test LIMIT 5")
    assert "estimated_cost_ms" in explain
    assert "filter_strategy" in explain
    assert "index_used" in explain
    assert "tree" in explain


def test_search_with_ef_and_search_ids(temp_db_path):
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("search_variants", dimension=4, metric="cosine")
    _seed_collection(collection)

    ef_results = collection.search_with_ef([1.0, 0.0, 0.0, 0.0], top_k=2, ef_search=64)
    id_results = collection.search_ids([1.0, 0.0, 0.0, 0.0], top_k=2)

    assert len(ef_results) <= 2
    assert len(id_results) <= 2
    if id_results:
        first = id_results[0]
        assert isinstance(first, dict)
        assert "id" in first
        assert "score" in first
