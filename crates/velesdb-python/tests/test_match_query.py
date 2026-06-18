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
    # 3 nodes seeded, LIMIT 2 must yield exactly 2 (exercises LIMIT enforcement in execute_match)
    assert len(results) == 2
    # returned node IDs must be the real seeded nodes, not garbage
    assert {r["node_id"] for r in results} <= {1, 2, 3}
    # score is None when no vector is provided; present as None or float
    for r in results:
        assert r["score"] is None or isinstance(r["score"], float)
        assert {"node_id", "depth", "path", "bindings", "score", "projected"} <= set(r.keys())


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
    # Query [1,0,0,0] is identical to node 1 (cosine=1.0) -> must rank first.
    assert results[0]["node_id"] == 1
    # Node 2 at [0,1,0,0] (cosine=0.0) is below threshold 0.5 and excluded;
    # only node 1 (1.0) and node 3 (~0.994) survive.
    assert {r["node_id"] for r in results} == {1, 3}
    # Every returned score is populated and at/above the 0.5 threshold.
    assert all(r.get("score") is not None for r in results)
    assert all(r["score"] >= 0.5 for r in results)
    # execute_match_with_similarity sorts cosine results descending by score.
    assert results[0]["score"] >= results[-1]["score"]


def test_match_query_invalid_non_match_query(temp_db_path):
    """`match_query` rejects non-MATCH statements with a typed `ValueError`.

    Routed through `core_err` since Wave 3 Commit 2 — `VELES-010 Query`
    is a query-shape error and surfaces as Python's canonical `ValueError`.
    """
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("match_invalid", dimension=4, metric="cosine")
    _seed_collection(collection)

    with pytest.raises(ValueError):
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

    assert len(ef_results) == 2
    assert len(id_results) == 2
    first, second = id_results[0], id_results[1]
    assert isinstance(first, dict)
    assert "id" in first and "score" in first
    # query vector == node 1's vector -> node 1 is the exact (cosine ~1.0) match
    assert first["id"] == 1
    # descending-score ordering (no ties: node 3 is strictly less similar)
    assert first["score"] > second["score"]
