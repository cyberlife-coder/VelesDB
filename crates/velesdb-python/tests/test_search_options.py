"""Tests for SearchOptions builder (issue #717 — v1.15 additive path).

Validates:
  - SearchOptions construction (defaults, fields)
  - search_request() produces same results as search()
  - search() emits no DeprecationWarning
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
    assert opts.k == 10
    assert opts.vector is None
    assert opts.sparse_vector is None
    assert opts.filter is None
    assert opts.sparse_index_name is None
    assert opts.include_vectors is False


def test_search_options_fields_set():
    vec = [0.1, 0.2, 0.3, 0.4]
    opts = SearchOptions(vector=vec, k=5, include_vectors=True)
    assert opts.k == 5
    assert opts.include_vectors is True
    assert list(opts.vector) == vec


def test_search_options_repr():
    opts = SearchOptions(k=20, include_vectors=True, sparse_index_name="my_idx")
    r = repr(opts)
    assert "SearchOptions" in r
    assert "20" in r
    assert "my_idx" in r


def test_search_options_has_no_top_k_alias():
    """``top_k`` was a deprecated alias of ``k``, removed in 8.0.0."""
    with pytest.raises(TypeError):
        SearchOptions(top_k=7)
    assert not hasattr(SearchOptions(), "with_top_k")
    assert SearchOptions().k == 10


# ---------------------------------------------------------------------------
# B. search_request() produces same results as search()
# ---------------------------------------------------------------------------


def test_search_request_matches_search(collection):
    query = [0.5, 0.5, 0.5, 0.1]

    shorthand = collection.search(vector=query, top_k=3)

    opts = SearchOptions(vector=query, k=3)
    modern = collection.search_request(opts)

    assert len(shorthand) == len(modern)
    for l, m in zip(shorthand, modern):
        assert l["id"] == m["id"]
        assert abs(l["score"] - m["score"]) < 1e-5


def test_search_request_with_filter(collection):
    query = [0.5, 0.5, 0.5, 0.1]
    opts = SearchOptions(
        vector=query,
        k=5,
        filter={"condition": {"type": "eq", "field": "idx", "value": 2}},
    )
    results = collection.search_request(opts)
    # Filter should restrict to the single point with idx=2
    for r in results:
        assert r["payload"]["idx"] == 2


def test_search_request_k_respected(collection):
    query = [0.5, 0.5, 0.5, 0.1]
    opts = SearchOptions(vector=query, k=2)
    results = collection.search_request(opts)
    assert len(results) == 2, f"k=2 over 5 points must return exactly 2 results, got {len(results)}"


# ---------------------------------------------------------------------------
# C. search() is a supported shorthand: it warns about nothing
# ---------------------------------------------------------------------------


def test_search_shorthand_emits_no_deprecation_warning(collection):
    query = [0.5, 0.5, 0.5, 0.1]
    with warnings.catch_warnings():
        warnings.simplefilter("error", DeprecationWarning)
        results = collection.search(vector=query, top_k=2)
    assert len(results) == 2
