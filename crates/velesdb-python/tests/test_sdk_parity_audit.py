"""Regression tests for the 2026-11 Python SDK parity audit (branch audit/python-sdk-parity).

Covers the three correctness gaps the multi-agent parity review found on the
search/ingest hot paths:

1. `upsert_bulk_numpy` / `upsert_bulk_numpy_json` accepted NaN/Inf vectors that
   every other ingest path rejects (persistent HNSW corruption). The core sink
   (`validate_raw_inputs`) now rejects them and the binding fails fast too.
2. Hybrid dense+sparse search with a caller filter post-filtered AFTER RRF
   fusion, so it could return fewer than k rows even when >= k points matched.
   The binding now uses core's `_filtered` leaf, which pre-filters both fusion
   branches — identical to the engine's own semantics.
3. MATCH results dropped `edge_bindings` / `edge_paths`, making variable-length
   relationship bindings (`-[r*1..3]->`) unreadable from Python. The converter
   now emits both maps, exactly like core's `MatchResult`.

Run with: pytest tests/test_sdk_parity_audit.py -v
"""

import json

import numpy as np
import pytest

from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS

try:
    import velesdb
except (ImportError, AttributeError):
    velesdb = None  # type: ignore[assignment]


# ---------------------------------------------------------------------------
# 1. Non-finite vectors on the numpy bulk paths
# ---------------------------------------------------------------------------


def test_upsert_bulk_numpy_rejects_nan(temp_db):
    col = temp_db.create_collection("bulk_nan", dimension=4, metric="cosine")
    vectors = np.array(
        [[1.0, 0.0, 0.0, 0.0], [0.0, float("nan"), 0.0, 0.0]], dtype=np.float32
    )
    with pytest.raises(ValueError, match="non-finite"):
        col.upsert_bulk_numpy(vectors, [1, 2])
    assert col.count() == 0, "a rejected batch must store nothing"


def test_upsert_bulk_numpy_rejects_infinity(temp_db):
    col = temp_db.create_collection("bulk_inf", dimension=4, metric="cosine")
    vectors = np.array([[float("inf"), 0.0, 0.0, 0.0]], dtype=np.float32)
    with pytest.raises(ValueError, match="non-finite"):
        col.upsert_bulk_numpy(vectors, [1])
    assert col.count() == 0


def test_upsert_bulk_numpy_json_rejects_nan(temp_db):
    col = temp_db.create_collection("bulk_nan_json", dimension=4, metric="cosine")
    vectors = np.array([[0.0, 0.0, float("nan"), 0.0]], dtype=np.float32)
    payloads = [json.dumps({"a": 1})]
    with pytest.raises(ValueError, match="non-finite"):
        col.upsert_bulk_numpy_json(vectors, [1], payloads)
    assert col.count() == 0


def test_upsert_bulk_numpy_finite_still_works(temp_db):
    col = temp_db.create_collection("bulk_ok", dimension=4, metric="cosine")
    vectors = np.array(
        [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]], dtype=np.float32
    )
    assert col.upsert_bulk_numpy(vectors, [1, 2]) == 2
    assert col.count() == 2
    results = col.search([1.0, 0.0, 0.0, 0.0], top_k=1)
    assert results[0]["id"] == 1


# ---------------------------------------------------------------------------
# 2. Hybrid dense+sparse filter pre-filters both fusion branches
# ---------------------------------------------------------------------------


def test_hybrid_sparse_filter_prefilters_before_fusion(tmp_path):
    """A filtered hybrid query must return k matching rows when >= k match.

    Point 1 dominates both branches but fails the filter, so an
    unfiltered-fused top-k of 3 contains it and post-filtering would leave
    only 2 rows. Core's `_filtered` leaf pre-filters both branches and
    returns all 3 matching rows.
    """
    db = velesdb.Database(str(tmp_path))
    col = db.create_collection("hybrid_prefilter", dimension=4, metric="cosine")
    col.upsert(
        [
            {
                "id": 1,
                "vector": [1.0, 0.0, 0.0, 0.0],
                "sparse_vector": {0: 10.0},
                "payload": {"keep": False},
            },
            {
                "id": 2,
                "vector": [0.9, 0.1, 0.0, 0.0],
                "sparse_vector": {0: 1.0},
                "payload": {"keep": True},
            },
            {
                "id": 3,
                "vector": [0.8, 0.2, 0.0, 0.0],
                "sparse_vector": {0: 0.9},
                "payload": {"keep": True},
            },
            {
                "id": 4,
                "vector": [0.7, 0.3, 0.0, 0.0],
                "sparse_vector": {0: 0.8},
                "payload": {"keep": True},
            },
        ]
    )
    filt = {"condition": {"type": "eq", "field": "keep", "value": True}}
    results = col.search_request(
        velesdb.SearchOptions(
            vector=[1.0, 0.0, 0.0, 0.0], sparse_vector={0: 10.0}, k=3, filter=filt
        )
    )
    assert len(results) == 3, (
        f"pre-filtered hybrid must return all 3 matching rows, got "
        f"{[r['id'] for r in results]}"
    )
    assert {r["id"] for r in results} == {2, 3, 4}


def test_hybrid_sparse_filter_sparse_only_still_rejected(tmp_path):
    """Sparse-only + caller filter keeps its loud error (no such core leaf)."""
    db = velesdb.Database(str(tmp_path))
    col = db.create_collection("sparse_only", dimension=4, metric="cosine")
    col.upsert(
        [{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "sparse_vector": {0: 1.0}}]
    )
    filt = {"condition": {"type": "eq", "field": "keep", "value": True}}
    with pytest.raises(ValueError, match="sparse-only"):
        col.search_request(
            velesdb.SearchOptions(sparse_vector={0: 1.0}, k=1, filter=filt)
        )


# ---------------------------------------------------------------------------
# 3. Graph parity: strict schema, edge reads, traversal options, fidelity
# ---------------------------------------------------------------------------


def _seed_graph(db, name="kg"):
    g = db.create_graph_collection(name)
    g.upsert_node_payload(10, {"_labels": ["Person"], "name": "Alice"})
    g.upsert_node_payload(20, {"_labels": ["Person"], "name": "Bob"})
    g.upsert_node_payload(30, {"_labels": ["Company"], "name": "Acme"})
    g.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})
    g.add_edge({"id": 2, "source": 20, "target": 30, "label": "WORKS_AT"})
    return g


def test_graph_strict_schema_declarable_from_python(tmp_path):
    """GraphSchema.strict() can now declare node/edge types (was hollow)."""
    db = velesdb.Database(str(tmp_path))
    schema = (
        velesdb.PyGraphSchema.strict()
        .with_node_type("Person", {"name": "string", "age": "integer"})
        .with_node_type("Company")
        .with_edge_type("KNOWS", "Person", "Person")
        .with_edge_type("WORKS_AT", "Person", "Company", {"since": "integer"})
    )
    g = db.create_graph_collection("strict_kg", schema=schema)

    read_back = g.schema
    assert read_back.is_schemaless is False
    node_types = {t["name"]: t for t in read_back.node_types}
    assert set(node_types) == {"Person", "Company"}
    assert node_types["Person"]["properties"] == {"name": "string", "age": "integer"}
    edge_types = {t["name"]: t for t in read_back.edge_types}
    assert edge_types["KNOWS"]["from_type"] == "Person"
    assert edge_types["WORKS_AT"]["to_type"] == "Company"
    assert edge_types["WORKS_AT"]["properties"] == {"since": "integer"}

    # Valid edges accepted, undeclared type rejected (VELES-017 -> ValueError).
    g.upsert_node_payload(10, {"_labels": ["Person"], "name": "Alice"})
    g.upsert_node_payload(20, {"_labels": ["Person"], "name": "Bob"})
    g.upsert_node_payload(30, {"_labels": ["Company"], "name": "Acme"})
    g.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})
    g.add_edge({"id": 2, "source": 20, "target": 30, "label": "WORKS_AT"})
    with pytest.raises(ValueError, match="not allowed"):
        g.add_edge({"id": 3, "source": 10, "target": 30, "label": "LIKES"})


def test_graph_schema_rejects_unknown_value_type():
    with pytest.raises(ValueError, match="unknown value type"):
        velesdb.PyGraphSchema.strict().with_node_type("T", {"x": "bogus"})


def test_graph_collection_edge_reads_and_mutations(tmp_path):
    db = velesdb.Database(str(tmp_path))
    g = _seed_graph(db)

    edge = g.get_edge(1)
    assert edge is not None and edge["label"] == "KNOWS" and edge["source"] == 10
    assert g.get_edge(999) is None

    bounded = g.get_outgoing_bounded(10, 1)
    assert bounded["total"] == 1 and len(bounded["edges"]) == 1
    bounded_in = g.get_incoming_bounded(20, 5)
    assert bounded_in["total"] == 1 and bounded_in["edges"][0]["id"] == 1

    # try_remove_edge: absent -> False, present -> True
    assert g.try_remove_edge(999) is False
    assert g.try_remove_edge(2) is True
    assert g.remove_edge(2) is False  # already gone
    assert g.edge_count() == 1

    # batch payload upsert + scroll_batch + metrics
    g.upsert_node_payloads([
        {"id": 40, "payload": {"_labels": ["Person"], "name": "Carol"}},
        {"id": 50, "payload": {"_labels": ["Person"], "name": "Dave"}},
    ])
    points, cursor = g.scroll_batch(batch_size=10)
    assert {p["id"] for p in points} == {10, 20, 30, 40, 50}
    assert cursor == 50  # last point's ID while a non-empty batch remains
    points2, cursor2 = g.scroll_batch(cursor=cursor, batch_size=10)
    assert points2 == [] and cursor2 is None
    m = g.metrics()
    assert m["edge_inserts_total"] >= 1 and m["edges_total"] >= 1

    edges_removed, nodes_removed = g.truncate()
    assert edges_removed == 1 and nodes_removed == 5
    assert g.edge_count() == 0 and g.count() == 0


def test_graph_traversal_min_depth_and_timeout(tmp_path):
    db = velesdb.Database(str(tmp_path))
    g = _seed_graph(db)
    full = g.traverse_bfs(10, max_depth=2)
    targets = {r["target_id"] for r in full}
    assert targets == {20, 30}
    # min_depth=2 skips the direct neighbour
    deep_only = g.traverse_bfs(10, max_depth=2, min_depth=2)
    assert {r["target_id"] for r in deep_only} == {30}
    # a zero timeout still returns a partial (possibly empty) result, no error
    partial = g.traverse_bfs(10, max_depth=2, timeout_ms=0)
    assert isinstance(partial, list)
    dfs = g.traverse_dfs(10, max_depth=2, min_depth=1, timeout_ms=1000)
    assert {r["target_id"] for r in dfs} == {20, 30}


def test_graph_store_direction_and_reads():
    store = velesdb.GraphStore()
    store.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})
    store.add_edge({"id": 2, "source": 20, "target": 30, "label": "WORKS_AT"})

    fwd = store.traverse_bfs(source=10, direction="forward")
    assert {r.target for r in fwd} == {20, 30}
    assert all(isinstance(r.path, list) and r.path for r in fwd)
    assert fwd[0].path == [1] or fwd[1].path == [1]

    rev = store.traverse_bfs(source=30, direction="reverse")
    assert {r.target for r in rev} == {20, 10}

    both = store.traverse_bfs(source=20, direction="both")
    assert {r.target for r in both} == {10, 30}

    with pytest.raises(Exception, match="unknown direction"):
        store.traverse_bfs(source=10, direction="sideways")

    assert store.get_edge(1)["label"] == "KNOWS"
    assert store.get_edge(99) is None
    assert len(store.all_edges()) == 2
    assert [e["id"] for e in store.get_incoming_by_label(20, "KNOWS")] == [1]
    assert store.out_degree(20) == 1 and store.in_degree(20) == 1

    streaming = store.traverse_bfs_streaming(
        10, velesdb.StreamingConfig(max_depth=2, max_visited=100, limit=1)
    )
    assert len(streaming) == 1  # explicit limit decoupled from max_visited

    assert store.remove_node_edges(20) == 2
    assert store.edge_count() == 0


# ---------------------------------------------------------------------------
# 4. Search / Database parity: tuning knobs, rrf_k, component_scores, explain
# ---------------------------------------------------------------------------


def _seed_docs(col):
    col.upsert(
        [
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"text": "machine learning"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"text": "deep learning"}},
            {"id": 3, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"text": "supervised learning"}},
        ]
    )


def test_search_options_ef_and_quality(temp_db):
    col = temp_db.create_collection("tune", dimension=4, metric="cosine")
    _seed_docs(col)
    r = col.search_request(velesdb.SearchOptions(vector=[1.0, 0, 0, 0], k=2, ef_search=64))
    assert len(r) == 2
    r = col.search_request(
        velesdb.SearchOptions(vector=[1.0, 0, 0, 0], k=2, quality="accurate")
    )
    assert len(r) == 2
    r = col.search_request(
        velesdb.SearchOptions().with_vector([1.0, 0, 0, 0]).with_k(2).with_quality("fast")
    )
    assert len(r) == 2
    # out-of-range ef is rejected client-side, same bounds as core
    with pytest.raises(ValueError, match="ef_search"):
        col.search_request(velesdb.SearchOptions(vector=[1.0, 0, 0, 0], k=2, ef_search=8))
    with pytest.raises(ValueError):
        col.search_request(
            velesdb.SearchOptions(vector=[1.0, 0, 0, 0], k=2, quality="bogus-mode")
        )


def test_hybrid_search_rrf_k_and_component_scores(temp_db):
    col = temp_db.create_collection("hyb", dimension=4, metric="cosine")
    _seed_docs(col)
    r = col.hybrid_search([1.0, 0, 0, 0], "learning", top_k=3, rrf_k=10)
    assert len(r) == 3
    assert r[0]["id"] == 1
    # the per-component breakdown core tags on hybrid paths is now visible
    assert "component_scores" in r[0]
    assert "vector_score" in r[0]["component_scores"]
    r2 = col.hybrid_search([1.0, 0, 0, 0], "learning", top_k=3, rrf_k=1)
    assert len(r2) == 3


def test_database_getters_guardrails_flush_all(tmp_path):
    db = velesdb.Database(str(tmp_path))
    assert db.data_dir == str(tmp_path)
    assert isinstance(db.schema_version(), int)
    limits = db.guardrail_limits()
    for key in (
        "max_depth",
        "max_cardinality",
        "memory_limit_bytes",
        "timeout_ms",
        "rate_limit_qps",
        "circuit_failure_threshold",
        "circuit_recovery_seconds",
    ):
        assert key in limits, f"guardrail_limits missing {key}"
    col = db.create_collection("c", dimension=4, metric="cosine")
    col.upsert([{"id": 1, "vector": [1.0, 0, 0, 0]}])
    assert db.flush_all() == 0


def test_database_explain_and_explain_analyze(tmp_path):
    db = velesdb.Database(str(tmp_path))
    col = db.create_collection("c", dimension=4, metric="cosine")
    _seed_docs(col)
    sql = "SELECT * FROM c WHERE vector NEAR $q LIMIT 3"
    # execute once so the compiled plan enters the cache
    db.execute_query(sql, params={"q": [1.0, 0, 0, 0]})
    plan = db.explain(sql, params={"q": [1.0, 0, 0, 0]})
    for key in ("tree", "estimated_cost_ms", "filter_strategy", "index_used", "cache_hit", "plan_reuse_count"):
        assert key in plan, f"explain plan missing {key}"
    assert plan["cache_hit"] is True  # the execute_query above cached it
    ea = db.explain_analyze("SELECT * FROM c LIMIT 3")
    assert ea["actual_stats"]["actual_rows"] == 3
    assert "plan" in ea and "node_stats" in ea


def test_train_quantizer_all_types_and_train_pq_compat(temp_db):
    col = temp_db.create_collection("q", dimension=8, metric="cosine")
    col.upsert(
        [
            {"id": i, "vector": [float((i >> j) & 1) for j in range(8)]}
            for i in range(1, 33)
        ]
    )
    assert "sq8" in temp_db.train_quantizer("q", quantizer="sq8")
    assert "rabitq" in temp_db.train_quantizer("q", quantizer="rabitq", force=True)
    with pytest.raises(ValueError, match="unknown quantizer"):
        temp_db.train_quantizer("q", quantizer="bogus")
    # train_pq keeps its historical message for pq/opq
    col2 = temp_db.create_collection("q2", dimension=8, metric="cosine")
    col2.upsert(
        [
            {"id": i, "vector": [float((i >> j) & 1) for j in range(8)]}
            for i in range(1, 33)
        ]
    )
    assert "PQ training complete" in temp_db.train_pq("q2", m=4, k=16)


def test_rebuild_index_and_all_point_ids(temp_db):
    col = temp_db.create_collection("ri", dimension=4, metric="cosine")
    col.upsert([{"id": i, "vector": [1.0, 0, 0, 0]} for i in range(1, 6)])
    col.delete([4, 5])
    reclaimed = col.rebuild_index()
    assert isinstance(reclaimed, int)
    assert sorted(col.all_point_ids()) == [1, 2, 3]
