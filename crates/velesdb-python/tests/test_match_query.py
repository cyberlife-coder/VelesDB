"""Tests for Collection.match_query() binding (Phase 4.3 Plan 01).

Validates that PyO3 match_query() correctly delegates to core's
execute_match() and execute_match_with_similarity().

Run with: pytest tests/test_match_query.py -v
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
def collection_with_graph():
    """Create a collection with vectors, edges, and labels for MATCH testing."""
    path = tempfile.mkdtemp(prefix="velesdb_match_test_")
    db = velesdb.Database(path)
    col = db.create_collection("test_match", dimension=4, metric="cosine")

    # Insert points with _labels for graph pattern matching
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
            {
                "id": 3,
                "vector": [0.0, 0.0, 1.0, 0.0],
                "payload": {"_labels": ["Document"], "title": "Paper"},
            },
        ]
    )

    # Add edges
    col.add_edge(100, 1, 2, "KNOWS")
    col.add_edge(101, 1, 3, "WROTE")
    col.add_edge(102, 2, 3, "REVIEWED")

    yield col
    shutil.rmtree(path, ignore_errors=True)


def test_match_query_basic(collection_with_graph):
    """Parse MATCH query, execute, verify result dict keys."""
    results = collection_with_graph.match_query(
        "MATCH (a:Person) RETURN a",
    )
    assert isinstance(results, list)
    # Should find Person nodes (Alice and Bob)
    assert len(results) >= 1

    # Verify all expected dict keys present
    for r in results:
        assert "node_id" in r
        assert "depth" in r
        assert "path" in r
        assert "bindings" in r
        assert "score" in r
        assert "projected" in r


def test_match_query_with_similarity(collection_with_graph):
    """Execute with vector + threshold, verify score is present."""
    results = collection_with_graph.match_query(
        "MATCH (a:Person) RETURN a",
        vector=[1.0, 0.0, 0.0, 0.0],
        threshold=0.0,
    )
    assert isinstance(results, list)
    assert len(results) >= 1
    for r in results:
        # Score must be set when vector is provided
        assert r["score"] is not None
        assert isinstance(r["score"], float)


def test_match_query_invalid_query(collection_with_graph):
    """Non-MATCH query raises error."""
    with pytest.raises(Exception):
        collection_with_graph.match_query(
            "SELECT * FROM test_match LIMIT 10"
        )


def test_match_query_empty_results(collection_with_graph):
    """Valid query with no matches returns empty list."""
    results = collection_with_graph.match_query(
        "MATCH (a:NonExistentLabel) RETURN a",
    )
    assert isinstance(results, list)
    assert len(results) == 0


def test_match_query_result_fields(collection_with_graph):
    """Verify all dict keys: node_id, depth, path, bindings, score, projected."""
    results = collection_with_graph.match_query(
        "MATCH (a:Person)-[:KNOWS]->(b) RETURN a, b",
    )
    assert isinstance(results, list)
    assert len(results) >= 1

    r = results[0]
    assert isinstance(r["node_id"], int)
    assert isinstance(r["depth"], int)
    assert isinstance(r["path"], list)
    assert isinstance(r["bindings"], dict)
    # score is None when no vector provided
    assert r["score"] is None
    assert isinstance(r["projected"], dict)
