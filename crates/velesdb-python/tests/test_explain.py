"""Tests for Collection.explain() binding (Phase 4.3 Plan 02).

Validates that PyO3 explain() correctly delegates to core's
QueryPlan::from_select() and QueryPlan::from_match().

Run with: pytest tests/test_explain.py -v
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
def collection_with_data():
    """Create a collection with vectors for EXPLAIN testing."""
    path = tempfile.mkdtemp(prefix="velesdb_explain_test_")
    db = velesdb.Database(path)
    col = db.create_collection("test_explain", dimension=4, metric="cosine")

    col.upsert(
        [
            {
                "id": 1,
                "vector": [1.0, 0.0, 0.0, 0.0],
                "payload": {"_labels": ["Person"], "name": "Alice"},
            },
            {
                "id": 2,
                "vector": [0.0, 1.0, 0.0, 0.0],
                "payload": {"_labels": ["Person"], "name": "Bob"},
            },
        ]
    )

    col.add_edge(100, 1, 2, "KNOWS")

    yield col
    shutil.rmtree(path, ignore_errors=True)


def test_explain_select_query(collection_with_data):
    """Explain a SELECT query, verify plan has expected keys."""
    plan = collection_with_data.explain(
        "SELECT * FROM test_explain LIMIT 10"
    )
    assert isinstance(plan, dict)
    assert "query_type" in plan
    assert plan["query_type"] == "SELECT"
    assert "estimated_cost_ms" in plan
    assert "index_used" in plan
    assert "filter_strategy" in plan


def test_explain_match_query(collection_with_data):
    """Explain a MATCH query, verify query_type is MATCH."""
    plan = collection_with_data.explain(
        "MATCH (a:Person)-[:KNOWS]->(b) RETURN a, b"
    )
    assert isinstance(plan, dict)
    assert plan["query_type"] == "MATCH"
    assert "estimated_cost_ms" in plan


def test_explain_vector_search_detected(collection_with_data):
    """SELECT with NEAR, verify index_used is set."""
    plan = collection_with_data.explain(
        "SELECT * FROM test_explain WHERE vector NEAR $v LIMIT 10"
    )
    assert isinstance(plan, dict)
    assert plan["index_used"] is not None


def test_explain_invalid_query(collection_with_data):
    """Syntax error raises exception."""
    with pytest.raises(Exception):
        collection_with_data.explain("SELEC * FROM test_explain")


def test_explain_returns_dict_keys(collection_with_data):
    """Verify keys: query_type, plan, estimated_cost_ms, index_used, filter_strategy."""
    plan = collection_with_data.explain(
        "SELECT * FROM test_explain LIMIT 10"
    )
    expected_keys = {"query_type", "plan", "estimated_cost_ms", "index_used", "filter_strategy"}
    assert expected_keys.issubset(set(plan.keys()))
