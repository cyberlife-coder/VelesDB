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
    graph.upsert_node_payload(10, {"_labels": ["Person"], "name": "Alice"})
    graph.upsert_node_payload(20, {"_labels": ["Person"], "name": "Bob"})
    graph.upsert_node_payload(30, {"_labels": ["Company"], "name": "Acme"})

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
    assert len(results) == 3, "all 3 seeded nodes should match unlabeled MATCH (n)"
    node_ids = {r["node_id"] for r in results}
    assert node_ids == {10, 20, 30}, f"expected the 3 seeded node IDs, got {node_ids}"


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
    # Exactly one edge of each type in the fixture, kept distinct by the filter
    assert len(knows) == 1
    assert len(works) == 1

    # KNOWS edge is Alice(10) -> Bob(20); terminal node_id is the b-binding (20)
    assert knows[0]["bindings"] == {"a": 10, "b": 20}
    assert knows[0]["node_id"] == 20

    # WORKS_AT edge is Bob(20) -> Acme(30); terminal node_id is 30
    assert works[0]["bindings"] == {"a": 20, "b": 30}
    assert works[0]["node_id"] == 30


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
    for key in (
        "node_id",
        "depth",
        "path",
        "bindings",
        "edge_bindings",
        "edge_paths",
        "score",
        "projected",
    ):
        assert key in r, f"Missing key '{key}' in match result"


def test_match_query_variable_length_edge_paths(graph_db):
    """Variable-length `-[r*1..2]->` must bind `r` to the ordered edge-id list.

    Core's `MatchResult` carries fixed-length aliases in `edge_bindings` and
    variable-length ones in `edge_paths` (openCypher list semantics); the
    Python converter used to drop both.
    """
    results = graph_db.match_query("MATCH (a)-[r*1..2]->(b) RETURN a, b, r LIMIT 10")
    assert len(results) > 0
    one_hop = [r for r in results if r["depth"] == 1]
    two_hop = [r for r in results if r["depth"] == 2]
    assert one_hop and two_hop, "seeded graph has 1-hop and 2-hop matches"
    # Variable-length alias: ordered edge-id list in edge_paths
    # (Alice -KNOWS(1)-> Bob, then Bob -WORKS_AT(2)-> Acme).
    assert any(r["edge_paths"].get("r") == [1] for r in one_hop)
    assert any(
        r["edge_paths"].get("r") == [1, 2] for r in two_hop
    ), f"expected edge_paths['r'] == [1, 2] on a 2-hop match, got {two_hop}"


def test_match_query_fixed_length_edge_bindings(graph_db):
    """A fixed-length `-[r]->` binds `r` to a scalar edge id in edge_bindings."""
    results = graph_db.match_query("MATCH (a)-[r:KNOWS]->(b) RETURN a, b, r LIMIT 10")
    assert len(results) > 0
    assert all(r["edge_bindings"].get("r") == 1 for r in results)


def test_explain_on_graph_collection(graph_db):
    """explain() should work on GraphCollection for MATCH queries."""
    plan = graph_db.explain("MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10")
    assert "estimated_cost_ms" in plan
    assert "tree" in plan


def test_query_method_on_graph_collection(graph_db):
    """query() (VelesQL SELECT) should work on GraphCollection."""
    results = graph_db.query("SELECT * FROM kg LIMIT 5")
    assert isinstance(results, list)
    assert len(results) == 3, "SELECT should return all 3 seeded nodes"
    ids = {r["id"] for r in results}
    assert ids == {10, 20, 30}
    assert all("payload" in r and "node_id" in r for r in results)


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
    bfs_targets = {r["target_id"] for r in bfs_results}
    match_nodes = {r["node_id"] for r in match_results}
    # BFS over KNOWS from Alice(10) and MATCH (a)-[:KNOWS]->(b) must both reach Bob(20)
    assert 20 in bfs_targets, f"BFS should reach Bob(20) via KNOWS, got {bfs_targets}"
    assert 20 in match_nodes, f"MATCH should reach Bob(20) via KNOWS, got {match_nodes}"
    assert bfs_targets & match_nodes, "BFS and MATCH must agree on at least one reached node"


def test_search_by_embedding_without_embeddings_raises_value_error(tmp_path):
    db = velesdb.Database(str(tmp_path))
    graph = db.create_graph_collection("no_emb")
    with pytest.raises(ValueError, match="VELES-014"):
        graph.search_by_embedding([1.0, 0.0, 0.0, 0.0])


def test_count_len_and_get_see_only_nodes_stored_with_a_vector(tmp_path):
    """What the docs now say: ``count``/``len``/``is_empty``/``get`` read the
    points stored with a vector; a payload-only node is listed by
    ``all_node_ids`` and ``in``, and read by ``get_node_payload``."""
    db = velesdb.Database(str(tmp_path))
    graph = db.create_graph_collection("vec_nodes", dimension=3)
    graph.upsert_node_payload(1, {"name": "payload only"})
    assert graph.count() == len(graph) == 0 and graph.is_empty()
    assert graph.get([1]) == [None]
    assert 1 in graph and graph.all_node_ids() == [1]
    assert graph.get_node_payload(1) == {"name": "payload only"}

    graph.upsert_node(2, {"name": "with vector"}, [1.0, 0.0, 0.0])
    assert graph.count() == len(graph) == 1 and not graph.is_empty()
    assert graph.get([2])[0]["payload"] == {"name": "with vector"}

    plain = db.create_graph_collection("no_vec_nodes")
    plain.upsert_node_payload(1, {"name": "payload only"})
    assert plain.count() == 0 and plain.get([1]) == [None] and 1 in plain


def test_query_runs_on_its_own_collection_whatever_params_collection_says(graph_db, tmp_path):
    """``GraphCollection.query`` always reads its own collection: a
    ``_collection`` parameter does not retarget it."""
    expected = graph_db.query("MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10")
    assert expected
    redirected = graph_db.query(
        "MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10",
        params={"_collection": "some_other_graph"},
    )
    assert [r["id"] for r in redirected] == [r["id"] for r in expected]
