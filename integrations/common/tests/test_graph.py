import pytest

from velesdb_common.graph import (
    build_graph_rest_payload,
    is_timeout_exception,
    open_native_graph,
    parse_graph_traverse_response,
    parse_graph_traverse_results,
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
    # Empty rel_types must be preserved verbatim (docstring: "empty list means
    # all types") — guard against a regression that drops the key or substitutes
    # a non-empty default like ["*"] / None.
    assert "rel_types" in payload
    assert payload["rel_types"] == []
    assert payload["rel_types"] is not None


def test_is_timeout_exception_with_timeout():
    exc = TimeoutError("Connection timed out")
    assert is_timeout_exception(exc) is True


def test_is_timeout_exception_with_other():
    exc = ValueError("Some error")
    assert is_timeout_exception(exc) is False


class _RecordingDatabase:
    """Recording fake for ``velesdb.Database`` capturing constructor calls."""

    calls = []

    def __init__(self, *args, **kwargs):
        type(self).calls.append((args, kwargs))

    def get_graph_collection(self, name):
        return object()


@pytest.fixture
def recording_db(monkeypatch):
    """Patch velesdb.Database with a recording fake and reset its call log."""
    velesdb = pytest.importorskip("velesdb")
    _RecordingDatabase.calls = []
    monkeypatch.setattr(velesdb, "Database", _RecordingDatabase)
    return _RecordingDatabase


def test_open_native_graph_forwards_config(tmp_path, recording_db):
    """Issue #1549: a provided config is forwarded verbatim to Database."""
    config = object()  # opaque pass-through payload
    graph = open_native_graph(str(tmp_path), "kg", config=config)
    assert graph is not None
    assert recording_db.calls == [((str(tmp_path),), {"config": config})]


def test_open_native_graph_no_config_call_unchanged(tmp_path, recording_db):
    """Without config the historical Database(path) call is preserved."""
    graph = open_native_graph(str(tmp_path), "kg")
    assert graph is not None
    assert recording_db.calls == [((str(tmp_path),), {})]


class _Response:
    """A ``requests.Response`` stand-in carrying a JSON body."""

    def __init__(self, status_code, body=None):
        self.status_code = status_code
        self._body = body

    def json(self):
        return self._body


# Shaped like the server's ``TraverseResponse``: ids serialised as strings.
_TRAVERSE_BODY = {
    "results": [
        {"target_id": "42", "depth": 1, "path": ["7"]},
        {"target_id": "43", "depth": 2, "path": ["7", "8"]},
    ],
    "has_more": False,
    "stats": {"visited": 3, "depth_reached": 2},
}


def test_given_server_traverse_response_when_parsed_then_int_ids_with_depth():
    assert parse_graph_traverse_results(_Response(200, _TRAVERSE_BODY)) == [(42, 1), (43, 2)]


def test_given_server_traverse_response_when_parsed_for_ids_then_ints():
    assert parse_graph_traverse_response(_Response(200, _TRAVERSE_BODY)) == [42, 43]


def test_given_error_status_when_parsed_then_no_results():
    assert parse_graph_traverse_results(_Response(404)) == []
