"""Tests for SearchOptions builder (issue #717 — v1.15 additive path).

Validates:
  - SearchOptions construction (defaults, fields)
  - search_request() produces same results as search()
  - DeprecationWarning emitted by legacy search()
  - __repr__ works

Date: 2026-05-09
"""

import warnings

import pytest

from tests.conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS

try:
    from velesdb import Database, SearchOptions

    VELESDB_AVAILABLE = True
except (ImportError, AttributeError):
    VELESDB_AVAILABLE = False
    SearchOptions = None  # type: ignore[assignment,misc]


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def collection(temp_db):
    """4-dim cosine collection with 5 pre-inserted points."""
    col = temp_db.create_collection("test_opts", dimension=4, metric="cosine")
    points = [
        {"id": i, "vector": [float(i) / 4, float(4 - i) / 4, 0.5, 0.1], "payload": {"idx": i}}
        for i in range(5)
    ]
    col.upsert(points)
    return col


# ---------------------------------------------------------------------------
# A. SearchOptions construction
# ---------------------------------------------------------------------------


def test_search_options_defaults():
    opts = SearchOptions()
    assert opts.top_k == 10
    assert opts.vector is None
    assert opts.sparse_vector is None
    assert opts.filter is None
    assert opts.sparse_index_name is None
    assert opts.include_vectors is False


def test_search_options_fields_set():
    vec = [0.1, 0.2, 0.3, 0.4]
    opts = SearchOptions(vector=vec, top_k=5, include_vectors=True)
    assert opts.top_k == 5
    assert opts.include_vectors is True
    assert list(opts.vector) == vec


def test_search_options_repr():
    opts = SearchOptions(top_k=20, include_vectors=True, sparse_index_name="my_idx")
    r = repr(opts)
    assert "SearchOptions" in r
    assert "20" in r
    assert "my_idx" in r


# ---------------------------------------------------------------------------
# B. search_request() produces same results as search()
# ---------------------------------------------------------------------------


def test_search_request_matches_search(collection):
    query = [0.5, 0.5, 0.5, 0.1]

    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        legacy = collection.search(vector=query, top_k=3)

    opts = SearchOptions(vector=query, top_k=3)
    modern = collection.search_request(opts)

    assert len(legacy) == len(modern)
    for l, m in zip(legacy, modern):
        assert l["id"] == m["id"]
        assert abs(l["score"] - m["score"]) < 1e-5


def test_search_request_with_filter(collection):
    query = [0.5, 0.5, 0.5, 0.1]
    opts = SearchOptions(
        vector=query,
        top_k=5,
        filter={"condition": {"type": "eq", "field": "idx", "value": 2}},
    )
    results = collection.search_request(opts)
    # Filter should restrict to the single point with idx=2
    for r in results:
        assert r["payload"]["idx"] == 2


def test_search_request_top_k_respected(collection):
    query = [0.5, 0.5, 0.5, 0.1]
    opts = SearchOptions(vector=query, top_k=2)
    results = collection.search_request(opts)
    assert len(results) == 2, f"top_k=2 over 5 points must return exactly 2 results, got {len(results)}"


# ---------------------------------------------------------------------------
# C. DeprecationWarning from legacy search()
# ---------------------------------------------------------------------------


def test_legacy_search_emits_deprecation_warning(collection):
    query = [0.5, 0.5, 0.5, 0.1]
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        collection.search(vector=query, top_k=2)

    deprecations = [w for w in caught if issubclass(w.category, DeprecationWarning)]
    assert deprecations, "Expected at least one DeprecationWarning from search()"
    msg = str(deprecations[0].message)
    assert "search_request" in msg or "deprecated" in msg.lower()
