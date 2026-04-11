"""Tests for PyGraphCollection VelesQL query methods (match_query, query, explain).

Validates that edges added via add_edge() are found by match_query() on the
same GraphCollection instance — the bug fixed in this patch.
"""

import pytest

from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS

try:
    import velesdb
except (ImportError, AttributeError):
    velesdb = None  # type: ignore[assignment]


@pytest.fixture
def graph_db(tmp_path):
    """Yield a Database with a seeded GraphCollection."""
    db = velesdb.Database(str(tmp_path))
    graph = db.create_graph_collection("kg", dimension=4)

    # Store node payloads (with _labels for MATCH label matching)
    graph.store_node_payload(10, {"_labels": ["Person"], "name": "Alice"})
    graph.store_node_payload(20, {"_labels": ["Person"], "name": "Bob"})
    graph.store_node_payload(30, {"_labels": ["Company"], "name": "Acme"})

    # Edges: Alice -KNOWS-> Bob, Bob -WORKS_AT-> Acme
    graph.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})
    graph.add_edge({"id": 2, "source": 20, "target": 30, "label": "WORKS_AT"})

    return graph


def test_match_query_finds_edges(graph_db):
    """match_query on GraphCollection must find edges added via add_edge."""
    results = graph_db.match_query("MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10")
    assert len(results) > 0, "match_query should find KNOWS edges"
    first = results[0]
    assert "node_id" in first
    assert "bindings" in first


def test_match_query_single_node_pattern(graph_db):
    """MATCH (n) without relationships should return nodes."""
    results = graph_db.match_query("MATCH (n) RETURN n LIMIT 5")
    assert len(results) > 0


def test_match_query_label_filter(graph_db):
    """MATCH (n:Person) should filter by label."""
    results = graph_db.match_query("MATCH (n:Person) RETURN n LIMIT 10")
    assert len(results) > 0
    # All matched nodes should be persons (id 10 or 20)
    for r in results:
        assert r["node_id"] in (10, 20)


def test_match_query_relationship_type_filter(graph_db):
    """MATCH with specific relationship type should filter correctly."""
    knows = graph_db.match_query(
        "MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10"
    )
    works = graph_db.match_query(
        "MATCH (a)-[:WORKS_AT]->(b) RETURN a, b LIMIT 10"
    )
    assert len(knows) > 0
    assert len(works) > 0


def test_match_query_returns_zero_for_missing_rel(graph_db):
    """MATCH with non-existent relationship type returns empty."""
    results = graph_db.match_query(
        "MATCH (a)-[:NONEXISTENT]->(b) RETURN a, b LIMIT 10"
    )
    assert len(results) == 0


def test_match_query_result_structure(graph_db):
    """Verify all expected keys in match_query result dicts."""
    results = graph_db.match_query("MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 1")
    assert len(results) > 0
    r = results[0]
    for key in ("node_id", "depth", "path", "bindings", "score", "projected"):
        assert key in r, f"Missing key '{key}' in match result"


def test_explain_on_graph_collection(graph_db):
    """explain() should work on GraphCollection for MATCH queries."""
    plan = graph_db.explain("MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10")
    assert "estimated_cost_ms" in plan
    assert "tree" in plan


def test_query_method_on_graph_collection(graph_db):
    """query() (VelesQL SELECT) should work on GraphCollection."""
    results = graph_db.query("SELECT * FROM kg LIMIT 5")
    # Should return nodes stored in the collection
    assert isinstance(results, list)


def test_match_query_rejects_non_match(graph_db):
    """match_query should reject SELECT queries with a typed `ValueError`.

    Passing a SELECT statement to `match_query` is a query-shape error
    (`VELES-010 Query`), which `core_err` routes to Python's canonical
    `ValueError` since Wave 3 Commit 2 — the previous `pytest.raises(Exception)`
    was too loose and would silently accept any regression.
    """
    with pytest.raises(ValueError):
        graph_db.match_query("SELECT * FROM kg LIMIT 1")


def test_bfs_and_match_agree(graph_db):
    """BFS traversal and MATCH query should find the same edges."""
    bfs_results = graph_db.traverse_bfs(10, max_depth=1, rel_types=["KNOWS"])
    match_results = graph_db.match_query(
        "MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10"
    )
    # Both should find the Alice->Bob KNOWS edge
    assert len(bfs_results) > 0, "BFS should find KNOWS edges"
    assert len(match_results) > 0, "MATCH should find KNOWS edges"
