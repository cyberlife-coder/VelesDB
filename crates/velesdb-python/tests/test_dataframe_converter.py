"""
Unit tests for dataframe_converter.py.

These tests do not require the native VelesDB bindings — they exercise the
pure-Python conversion helpers directly.

Key-collision regression (Devin Review bugs):
  - to_dataframe: payload keys 'id'/'score' must NOT overwrite special columns
  - to_scroll_dataframe: payload keys 'id'/'vector' must NOT overwrite special columns

Run with: pytest tests/test_dataframe_converter.py -v
"""

from __future__ import annotations

from typing import Any

import pytest

from velesdb.dataframe_converter import (
    query_to_dataframe,
    to_dataframe,
    to_scroll_dataframe,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_search_result(
    point_id: int,
    score: float,
    payload: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {"id": point_id, "score": score, "payload": payload}


def _make_scroll_point(
    point_id: int,
    vector: list[float],
    payload: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {"id": point_id, "vector": vector, "payload": payload}


# ---------------------------------------------------------------------------
# to_dataframe — key-collision tests (core regression)
# ---------------------------------------------------------------------------

class TestToDataframeKeyCollision:
    """Payload keys that collide with 'id'/'score' must not overwrite them."""

    @pytest.fixture
    def pd(self):
        return pytest.importorskip("pandas")

    def test_payload_id_does_not_overwrite_result_id(self, pd):
        """A payload key named 'id' must not replace the search result id."""
        results = [
            _make_search_result(42, 0.9, payload={"id": 999, "label": "foo"}),
        ]
        df = to_dataframe(results, backend="pandas")
        assert df["id"].iloc[0] == 42, (
            f"Expected id=42, got id={df['id'].iloc[0]!r}. "
            "Payload 'id' overwrote the search result id."
        )

    def test_payload_score_does_not_overwrite_result_score(self, pd):
        """A payload key named 'score' must not replace the search result score."""
        results = [
            _make_search_result(1, 0.85, payload={"score": 0.0, "text": "hi"}),
        ]
        df = to_dataframe(results, backend="pandas")
        assert abs(df["score"].iloc[0] - 0.85) < 1e-9, (
            f"Expected score=0.85, got score={df['score'].iloc[0]!r}. "
            "Payload 'score' overwrote the search result score."
        )

    def test_both_collision_keys_in_payload(self, pd):
        """Both 'id' and 'score' payload keys must not overwrite special columns."""
        results = [
            _make_search_result(7, 0.77, payload={"id": 0, "score": -1.0}),
        ]
        df = to_dataframe(results, backend="pandas")
        assert df["id"].iloc[0] == 7
        assert abs(df["score"].iloc[0] - 0.77) < 1e-9

    def test_payload_extra_fields_are_kept(self, pd):
        """Non-colliding payload fields must appear as columns."""
        results = [
            _make_search_result(1, 0.5, payload={"category": "tech", "rank": 3}),
        ]
        df = to_dataframe(results, backend="pandas")
        assert "category" in df.columns
        assert df["category"].iloc[0] == "tech"
        assert df["rank"].iloc[0] == 3

    def test_no_payload(self, pd):
        """Results without payload must still produce id and score columns."""
        results = [_make_search_result(10, 0.6)]
        df = to_dataframe(results, backend="pandas")
        assert df["id"].iloc[0] == 10
        assert abs(df["score"].iloc[0] - 0.6) < 1e-9

    def test_empty_results_returns_empty_dataframe(self, pd):
        """Empty input returns an empty DataFrame with id and score columns."""
        df = to_dataframe([], backend="pandas")
        assert list(df.columns) == ["id", "score"]
        assert len(df) == 0

    def test_multiple_rows_collision(self, pd):
        """All rows must preserve result id/score despite payload collision."""
        results = [
            _make_search_result(1, 0.9, payload={"id": 100, "score": -5.0}),
            _make_search_result(2, 0.8, payload={"id": 200, "score": -6.0}),
            _make_search_result(3, 0.7, payload={"id": 300, "score": -7.0}),
        ]
        df = to_dataframe(results, backend="pandas")
        assert list(df["id"]) == [1, 2, 3]
        for i, expected_score in enumerate([0.9, 0.8, 0.7]):
            assert abs(df["score"].iloc[i] - expected_score) < 1e-9


# ---------------------------------------------------------------------------
# to_scroll_dataframe — key-collision tests (core regression)
# ---------------------------------------------------------------------------

class TestToScrollDataframeKeyCollision:
    """Payload keys that collide with 'id'/'vector' must not overwrite them."""

    @pytest.fixture
    def pd(self):
        return pytest.importorskip("pandas")

    def test_payload_id_does_not_overwrite_point_id(self, pd):
        """A payload key named 'id' must not replace the scroll point id."""
        batch = [
            _make_scroll_point(42, [0.1, 0.2], payload={"id": 999, "label": "foo"}),
        ]
        df = to_scroll_dataframe(batch, backend="pandas")
        assert df["id"].iloc[0] == 42, (
            f"Expected id=42, got id={df['id'].iloc[0]!r}. "
            "Payload 'id' overwrote the point id."
        )

    def test_payload_vector_does_not_overwrite_point_vector(self, pd):
        """A payload key named 'vector' must not replace the point's vector."""
        batch = [
            _make_scroll_point(1, [0.1, 0.2], payload={"vector": [9.9, 8.8]}),
        ]
        df = to_scroll_dataframe(batch, backend="pandas")
        assert df["vector"].iloc[0] == [0.1, 0.2], (
            f"Expected vector=[0.1, 0.2], got {df['vector'].iloc[0]!r}. "
            "Payload 'vector' overwrote the point vector."
        )

    def test_both_collision_keys_in_payload(self, pd):
        """Both 'id' and 'vector' in payload must not overwrite special columns."""
        batch = [
            _make_scroll_point(5, [1.0, 2.0], payload={"id": 0, "vector": []}),
        ]
        df = to_scroll_dataframe(batch, backend="pandas")
        assert df["id"].iloc[0] == 5
        assert df["vector"].iloc[0] == [1.0, 2.0]

    def test_non_dict_payload_stored_as_payload_column(self, pd):
        """When payload is not a dict, it is stored under the 'payload' column."""
        batch = [
            _make_scroll_point(1, [0.5], payload=None),
        ]
        df = to_scroll_dataframe(batch, backend="pandas")
        assert "payload" in df.columns
        assert df["payload"].iloc[0] is None

    def test_dict_payload_fields_expanded_as_columns(self, pd):
        """Dict payload fields must become their own columns."""
        batch = [
            _make_scroll_point(1, [0.1], payload={"tag": "news", "weight": 1.5}),
        ]
        df = to_scroll_dataframe(batch, backend="pandas")
        assert "tag" in df.columns
        assert df["tag"].iloc[0] == "news"
        assert abs(df["weight"].iloc[0] - 1.5) < 1e-9

    def test_empty_batch_returns_empty_dataframe(self, pd):
        """Empty input returns an empty DataFrame with id, vector, payload columns."""
        df = to_scroll_dataframe([], backend="pandas")
        assert "id" in df.columns
        assert "vector" in df.columns
        assert "payload" in df.columns
        assert len(df) == 0

    def test_multiple_rows_collision(self, pd):
        """All rows must preserve point id/vector despite payload collision."""
        batch = [
            _make_scroll_point(10, [1.0], payload={"id": 99, "vector": [0.0]}),
            _make_scroll_point(20, [2.0], payload={"id": 88, "vector": [0.0]}),
        ]
        df = to_scroll_dataframe(batch, backend="pandas")
        assert list(df["id"]) == [10, 20]
        assert df["vector"].iloc[0] == [1.0]
        assert df["vector"].iloc[1] == [2.0]


# ---------------------------------------------------------------------------
# query_to_dataframe — basic sanity (no collision expected, no special keys)
# ---------------------------------------------------------------------------

class TestQueryToDataframe:
    """Smoke tests for query_to_dataframe (no collision risk — no special keys)."""

    @pytest.fixture
    def pd(self):
        return pytest.importorskip("pandas")

    def test_basic_rows(self, pd):
        rows = [{"a": 1, "b": "x"}, {"a": 2, "b": "y"}]
        df = query_to_dataframe(rows, backend="pandas")
        assert list(df.columns) == ["a", "b"]
        assert len(df) == 2

    def test_empty_returns_empty_dataframe(self, pd):
        df = query_to_dataframe([], backend="pandas")
        assert len(df) == 0
