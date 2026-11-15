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
