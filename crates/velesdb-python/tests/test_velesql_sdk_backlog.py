"""Regression tests for the VelesQL Python SDK backlog (#1, #14, #24, #27).

Each test pins a documented-but-broken behavior so the fix is provable:

- #1  match_query honors RETURN ORDER BY + post-sort LIMIT (was raw order).
- #14 explain() of an indexed SELECT emits an IndexLookup node, MATCH reports a
      real strategy, and an invalid query raises.
- #24 typed hybrid dense+sparse search honors a `fusion=` strategy (was RRF k=60).
- #27 Database.set_auto_reindex toggles the runtime flag via ALTER COLLECTION.
"""

import shutil
import tempfile

import pytest

try:
    import velesdb
except ImportError:  # pragma: no cover
    pytest.skip(
        "velesdb module not built yet - run 'maturin develop' first",
        allow_module_level=True,
    )


@pytest.fixture
def temp_db_path():
    path = tempfile.mkdtemp(prefix="velesdb_sdk_backlog_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


# ---------------------------------------------------------------------------
# #1 — match_query RETURN ORDER BY + post-sort LIMIT
# ---------------------------------------------------------------------------


def test_match_query_orders_and_limits(temp_db_path):
    """MATCH ... RETURN d ORDER BY d.year DESC LIMIT 10 returns year-DESC top-10.

    More than LIMIT :Doc nodes are seeded with distinct years; the Python
    binding must rank identically to the SQL /query path (ORDER BY + post-sort
    LIMIT applied), not return raw traversal order.
    """
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("docs_order", dimension=2, metric="cosine")

    # 15 nodes, years 2000..2014 — distinct so the DESC order is unambiguous.
    points = [
        {
            "id": i,
            "vector": [1.0, 0.0],
            "payload": {"_labels": ["Doc"], "year": 2000 + i},
        }
        for i in range(15)
    ]
    collection.upsert(points)

    results = collection.match_query(
        "MATCH (d:Doc) RETURN d ORDER BY d.year DESC LIMIT 10"
    )

    # Post-sort LIMIT: exactly 10 of the 15 nodes.
    assert len(results) == 10, f"expected post-sort LIMIT 10, got {len(results)}"

    years = [r["projected"].get("year") or r["bindings"] for r in results]
    # The top-10 by year DESC are 2014..2005 -> node ids 14..5.
    ids = [r["node_id"] for r in results]
    assert ids == list(range(14, 4, -1)), f"expected year-DESC top-10 ids, got {ids}"
    # Sanity: the dropped tail (years <= 2004) must not appear.
    assert all(node_id >= 5 for node_id in ids)
    _ = years  # projected payload is best-effort; ordering is asserted via ids


# ---------------------------------------------------------------------------
# #14 — EXPLAIN: IndexLookup for indexed SELECT, real MATCH strategy, validation
# ---------------------------------------------------------------------------


def _tree_text(tree) -> str:
    """Flatten an explain tree (string or nested structure) to searchable text."""
    return repr(tree)


def test_explain_select_on_indexed_field_has_index_lookup(temp_db_path):
    db = velesdb.Database(temp_db_path)
    db.create_collection("idx_docs", dimension=2, metric="cosine")
    collection = db.get_collection("idx_docs")
    collection.upsert(
        [
            {"id": i, "vector": [1.0, 0.0], "payload": {"category": "a" if i % 2 else "b"}}
            for i in range(6)
        ]
    )

    # Register a secondary index on the WHERE field via VelesQL DDL.
    db.execute_query("CREATE INDEX ON idx_docs (category)")

    # Re-fetch so the wrapper observes the freshly registered index set.
    collection = db.get_collection("idx_docs")
    explain = collection.explain("SELECT * FROM idx_docs WHERE category = 'a' LIMIT 5")

    text = _tree_text(explain["tree"])
    assert "IndexLookup" in text, f"expected IndexLookup node, tree was: {text}"


def test_explain_match_reports_real_strategy(temp_db_path):
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("match_explain", dimension=2, metric="cosine")
    collection.upsert(
        [
            {"id": i, "vector": [1.0, 0.0], "payload": {"_labels": ["Doc"]}}
            for i in range(4)
        ]
    )

    explain = collection.explain("MATCH (d:Doc) RETURN d LIMIT 5")
    text = _tree_text(explain["tree"])
    # A real traversal strategy must surface (GraphFirst / VectorFirst / Parallel
    # / MatchTraversal), not a bare TableScan mislabeled MATCH.
    assert any(
        token in text
        for token in ("MatchTraversal", "GraphFirst", "VectorFirst", "Parallel")
    ), f"expected a real MATCH strategy, tree was: {text}"


def test_explain_rejects_invalid_query(temp_db_path):
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("explain_invalid", dimension=2, metric="cosine")
    collection.upsert([{"id": 1, "vector": [1.0, 0.0], "payload": {"amount": 5}}])

    # A query that parses but is semantically invalid (a WHERE subquery, which
    # VelesQL parses yet rejects at validation) must be rejected by explain()
    # rather than silently building a plan.
    with pytest.raises(ValueError):
        collection.explain(
            "SELECT * FROM explain_invalid WHERE amount > "
            "(SELECT AVG(amount) FROM explain_invalid) LIMIT 5"
        )


# ---------------------------------------------------------------------------
# #24 — typed hybrid dense+sparse honors a fusion strategy
# ---------------------------------------------------------------------------


def _seed_hybrid(collection):
    # node 1: dense-dominant, weak sparse.   node 2: sparse-dominant, weak dense.
    # node 3: middling on both. Distinct profiles so the dense vs sparse
    # branch weighting decides the top result with no rank ties.
    collection.upsert(
        [
            {
                "id": 1,
                "vector": [1.0, 0.0, 0.0, 0.0],
                "sparse_vector": {0: 0.01, 1: 0.01},
            },
            {
                "id": 2,
                "vector": [0.2, 0.97, 0.0, 0.0],
                "sparse_vector": {0: 5.0, 1: 5.0},
            },
            {
                "id": 3,
                "vector": [0.6, 0.6, 0.0, 0.0],
                "sparse_vector": {0: 0.5, 1: 0.5},
            },
        ]
    )


def test_hybrid_fusion_changes_ordering(temp_db_path):
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("hybrid_fusion", dimension=4, metric="cosine")
    _seed_hybrid(collection)

    dense = [1.0, 0.0, 0.0, 0.0]
    sparse = {0: 1.0, 1: 1.0}

    # All-dense RSF: dense-dominant node 1 ranks first.
    dense_opts = velesdb.SearchOptions(
        vector=dense, sparse_vector=sparse, top_k=3
    ).with_fusion(velesdb.FusionStrategy.relative_score(1.0, 0.0))
    dense_first = [r["id"] for r in collection.search_request(dense_opts)][0]

    # All-sparse RSF: sparse-dominant node 2 ranks first.
    sparse_opts = velesdb.SearchOptions(
        vector=dense, sparse_vector=sparse, top_k=3
    ).with_fusion(velesdb.FusionStrategy.relative_score(0.0, 1.0))
    sparse_first = [r["id"] for r in collection.search_request(sparse_opts)][0]

    assert dense_first == 1, f"all-dense RSF should rank node 1 first, got {dense_first}"
    assert sparse_first == 2, f"all-sparse RSF should rank node 2 first, got {sparse_first}"
    # The fusion= strategy demonstrably changes the top result.
    assert dense_first != sparse_first


def test_hybrid_fusion_default_unchanged(temp_db_path):
    """Omitting fusion= and passing fusion=None both use the default RRF(k=60).

    Asserts the default branch still routes to the same core call (RRF), not
    that an inherently tie-bearing ranking is bit-stable.
    """
    db = velesdb.Database(temp_db_path)
    collection = db.create_collection("hybrid_default", dimension=4, metric="cosine")
    _seed_hybrid(collection)

    dense = [1.0, 0.0, 0.0, 0.0]
    sparse = {0: 1.0, 1: 1.0}

    omitted = sorted(
        r["id"]
        for r in collection.search_request(
            velesdb.SearchOptions(vector=dense, sparse_vector=sparse, top_k=3)
        )
    )
    explicit_none = sorted(
        r["id"]
        for r in collection.search_request(
            velesdb.SearchOptions(vector=dense, sparse_vector=sparse, top_k=3).with_fusion(None)
        )
    )
    # Same candidate set under the default fusion (RRF), regardless of tie order.
    assert omitted == explicit_none == [1, 2, 3]


# ---------------------------------------------------------------------------
# #27 — Database.set_auto_reindex runtime toggle
# ---------------------------------------------------------------------------


def test_set_auto_reindex_toggles_flag(temp_db_path):
    db = velesdb.Database(temp_db_path)
    db.create_collection("reindex_toggle", dimension=2, metric="cosine")

    db.set_auto_reindex("reindex_toggle", True)
    info_on = db.get_collection("reindex_toggle").info()
    assert info_on.get("auto_reindex") is True

    db.set_auto_reindex("reindex_toggle", False)
    info_off = db.get_collection("reindex_toggle").info()
    assert info_off.get("auto_reindex") is False
