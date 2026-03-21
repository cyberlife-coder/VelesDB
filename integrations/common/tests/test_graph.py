import pytest
from velesdb_common.graph import (
    build_graph_rest_payload,
    is_timeout_exception,
)


def test_build_graph_rest_payload_basic():
    payload = build_graph_rest_payload("node-1", max_depth=3, expand_k=10, rel_types=[])
    assert payload["source"] == "node-1"
    assert payload["max_depth"] == 3
    assert payload["limit"] == 20  # expand_k * 2


def test_build_graph_rest_payload_with_rel_types():
    payload = build_graph_rest_payload(
        "node-1", max_depth=2, expand_k=5, rel_types=["KNOWS", "LIKES"]
    )
    assert payload["rel_types"] == ["KNOWS", "LIKES"]


def test_build_graph_rest_payload_empty_rel_types():
    payload = build_graph_rest_payload("node-1", max_depth=1, expand_k=10, rel_types=[])
    assert payload["rel_types"] == []


def test_is_timeout_exception_with_timeout():
    exc = TimeoutError("Connection timed out")
    assert is_timeout_exception(exc) is True


def test_is_timeout_exception_with_other():
    exc = ValueError("Some error")
    assert is_timeout_exception(exc) is False
