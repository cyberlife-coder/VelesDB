"""Tests for ``velesdb_common.aggregates``: an aggregate's rows are told
apart from points."""

from velesdb_common.aggregates import is_aggregate_row


def test_a_point_row_is_no_aggregate_row():
    point = {
        "node_id": 1, "vector_score": 1.0, "graph_score": None,
        "fused_score": 1.0, "bindings": {"cat": "x"}, "column_data": None,
        "id": 1, "score": 1.0, "payload": {"cat": "x"},
    }
    assert not is_aggregate_row(point)


def test_an_aggregate_row_carries_its_own_columns():
    assert is_aggregate_row({"count": 2})
    assert is_aggregate_row({"cat": "a", "count": 3})
    # A group keyed by a column named like a point field is still a group.
    assert is_aggregate_row({"id": 7, "count": 1})
