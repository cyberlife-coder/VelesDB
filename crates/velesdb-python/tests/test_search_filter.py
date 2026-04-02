"""
Regression tests for issue #479: search() silently ignored non-like/ilike filters.

Root cause: the `filter` kwarg was never forwarded to the Rust binding.
Instead, a Python-only `_matches_filter()` post-filter only handled
LIKE/ILIKE and returned True (pass-through) for all other operators.

These tests verify that EQ, GT, IN, and compound AND filters work
correctly end-to-end through the Rust binding.

Run with: pytest tests/test_search_filter.py -v
"""

import pytest

from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS

try:
    import velesdb
except (ImportError, AttributeError):
    velesdb = None  # type: ignore[assignment]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_collection(temp_db):
    """Create a 4-dim cosine collection with 5 points across categories."""
    col = temp_db.create_collection("filter_test", dimension=4, metric="cosine")
    col.upsert([
        {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"category": "A", "price": 10.0}},
        {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"category": "A", "price": 20.0}},
        {"id": 3, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"category": "B", "price": 60.0}},
        {"id": 4, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"category": "C", "price": 80.0}},
        {"id": 5, "vector": [0.0, 0.0, 0.0, 1.0], "payload": {"category": "B", "price": 90.0}},
    ])
    return col


# ---------------------------------------------------------------------------
# Tests — each covers one filter operator that was previously silently ignored
# ---------------------------------------------------------------------------

class TestSearchWithEqFilter:
    """EQ filter must restrict results to matching category."""

    def test_search_with_eq_filter_returns_only_matching(self, temp_db):
        """GIVEN 5 points, WHEN searching with eq category=A, THEN only A IDs returned."""
        col = _make_collection(temp_db)

        results = col.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={"must": [{"key": "category", "match": {"value": "A"}}]},
        )

        ids = {r["id"] for r in results}
        assert ids == {1, 2}, (
            f"Expected only IDs 1 and 2 (category=A), got {ids}. "
            "This indicates the filter was not forwarded to Rust."
        )

    def test_search_with_eq_filter_excludes_other_categories(self, temp_db):
        """No result with category != A must slip through the eq filter."""
        col = _make_collection(temp_db)

        results = col.search(
            [0.0, 1.0, 0.0, 0.0],
            top_k=10,
            filter={"must": [{"key": "category", "match": {"value": "A"}}]},
        )

        for r in results:
            assert r.get("payload", {}).get("category") == "A", (
                f"Result id={r['id']} has category={r.get('payload', {}).get('category')!r}, "
                "expected 'A' — filter not applied by Rust."
            )


class TestSearchWithGtFilter:
    """GT filter must restrict results to price > threshold."""

    def test_search_with_gt_filter_returns_only_above_threshold(self, temp_db):
        """GIVEN 5 points, WHEN searching with gt price>50, THEN only high-price IDs returned."""
        col = _make_collection(temp_db)

        results = col.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={"must": [{"key": "price", "range": {"gt": 50.0}}]},
        )

        ids = {r["id"] for r in results}
        assert ids == {3, 4, 5}, (
            f"Expected IDs 3, 4, 5 (price>50), got {ids}. "
            "GT filter was silently ignored before fix."
        )

    def test_search_with_gt_filter_no_results_above_max(self, temp_db):
        """GT filter above max price must return empty results."""
        col = _make_collection(temp_db)

        results = col.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={"must": [{"key": "price", "range": {"gt": 999.0}}]},
        )

        assert results == [], (
            f"Expected empty results for price>999, got {results}."
        )


class TestSearchWithInFilter:
    """IN filter must restrict to a set of allowed values."""

    def test_search_with_in_filter_returns_matching_categories(self, temp_db):
        """GIVEN 5 points, WHEN filtering category in [A, B], THEN only A/B IDs returned."""
        col = _make_collection(temp_db)

        results = col.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={"must": [{"key": "category", "match": {"any": ["A", "B"]}}]},
        )

        ids = {r["id"] for r in results}
        assert ids == {1, 2, 3, 5}, (
            f"Expected IDs 1, 2, 3, 5 (category in [A, B]), got {ids}."
        )

    def test_search_with_in_filter_excludes_unlisted_values(self, temp_db):
        """No result with category C must appear when filtering for [A, B]."""
        col = _make_collection(temp_db)

        results = col.search(
            [0.0, 0.0, 1.0, 0.0],
            top_k=10,
            filter={"must": [{"key": "category", "match": {"any": ["A", "B"]}}]},
        )

        for r in results:
            assert r.get("payload", {}).get("category") in {"A", "B"}, (
                f"Result id={r['id']} has category={r.get('payload', {}).get('category')!r}, "
                "expected one of A or B."
            )


class TestSearchWithCompoundAndFilter:
    """AND (must) compound filter combining EQ + GT must restrict by both conditions."""

    def test_search_with_and_eq_gt_returns_intersection(self, temp_db):
        """GIVEN 5 points, WHEN filtering category=B AND price>70, THEN only id=5 returned."""
        col = _make_collection(temp_db)

        results = col.search(
            [0.0, 1.0, 0.0, 0.0],
            top_k=10,
            filter={
                "must": [
                    {"key": "category", "match": {"value": "B"}},
                    {"key": "price", "range": {"gt": 70.0}},
                ]
            },
        )

        ids = {r["id"] for r in results}
        assert ids == {5}, (
            f"Expected only id=5 (category=B AND price>70), got {ids}."
        )

    def test_search_with_and_filter_empty_when_no_match(self, temp_db):
        """AND filter with impossible conjunction must return empty results."""
        col = _make_collection(temp_db)

        results = col.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={
                "must": [
                    {"key": "category", "match": {"value": "A"}},
                    {"key": "price", "range": {"gt": 999.0}},
                ]
            },
        )

        assert results == [], (
            f"Expected empty results for impossible conjunction, got {results}."
        )


class TestSearchFilterNoRegression:
    """Ensure existing behaviour (no filter) is unaffected by the fix."""

    def test_search_without_filter_returns_all_top_k(self, temp_db):
        """Unfiltered search must still return top_k results from all points."""
        col = _make_collection(temp_db)

        results = col.search([1.0, 0.0, 0.0, 0.0], top_k=5)

        assert len(results) == 5, (
            f"Expected 5 results (all points), got {len(results)}."
        )

    def test_search_filter_none_is_unfiltered(self, temp_db):
        """Explicitly passing filter=None must behave the same as omitting filter."""
        col = _make_collection(temp_db)

        results = col.search([1.0, 0.0, 0.0, 0.0], top_k=5, filter=None)

        assert len(results) == 5, (
            f"Expected 5 results with filter=None, got {len(results)}."
        )
