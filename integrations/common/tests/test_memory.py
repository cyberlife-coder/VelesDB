import pytest

from velesdb_common.memory import (
    chronological,
    format_procedural_results,
    parse_event_entry,
    resolve_procedure_id,
)


def test_format_procedural_results_basic():
    results = [
        {"id": 7, "name": "proc1", "steps": ["a", "b"], "confidence": 0.9, "score": 0.85},
    ]
    formatted = format_procedural_results(results)
    assert len(formatted) == 1
    assert formatted[0]["id"] == 7
    assert formatted[0]["name"] == "proc1"
    assert formatted[0]["steps"] == ["a", "b"]
    assert formatted[0]["confidence"] == 0.9
    assert formatted[0]["score"] == 0.85


def test_format_procedural_results_empty():
    assert format_procedural_results([]) == []


def test_format_procedural_results_preserves_id_for_reinforce():
    """The id must survive projection so reinforce() can use a recalled id."""
    raw = [{"id": 42, "name": "p", "steps": ["s"], "confidence": 0.5, "score": 0.7}]
    assert format_procedural_results(raw)[0]["id"] == 42


def test_resolve_procedure_id_by_int_id():
    """A numeric id is returned verbatim, even with an empty name map."""
    assert resolve_procedure_id(42, {}) == 42


def test_resolve_procedure_id_by_name():
    assert resolve_procedure_id("deploy", {"deploy": 5}) == 5


def test_resolve_procedure_id_unknown_name_raises():
    with pytest.raises(KeyError, match="ghost"):
        resolve_procedure_id("ghost", {"deploy": 5})


def test_resolve_procedure_id_bool_not_treated_as_id():
    """bool is an int subclass; it must not slip through as a valid id."""
    with pytest.raises(KeyError):
        resolve_procedure_id(True, {})


def test_parse_event_entry_valid_json():
    assert parse_event_entry('{"role": "ai", "content": "hi"}') == ("ai", "hi")


def test_parse_event_entry_malformed_defaults_to_human():
    assert parse_event_entry("not json") == ("human", "not json")


def test_parse_event_entry_non_object_defaults_to_human():
    assert parse_event_entry("[1, 2]") == ("human", "[1, 2]")


def test_chronological_reverses_newest_first_to_oldest_first():
    newest_first = [
        {"id": 3, "timestamp": 30},
        {"id": 2, "timestamp": 20},
        {"id": 1, "timestamp": 10},
    ]
    out = chronological(newest_first)
    assert [e["timestamp"] for e in out] == [10, 20, 30]


def test_chronological_breaks_timestamp_ties_by_id():
    """Events sharing one timestamp bucket must order by ascending id, not
    by the descending-id order VelesDB returns them in."""
    same_second = [
        {"id": 3, "timestamp": 100},
        {"id": 1, "timestamp": 100},
        {"id": 2, "timestamp": 100},
    ]
    out = chronological(same_second)
    assert [e["id"] for e in out] == [1, 2, 3]


def test_chronological_does_not_mutate_input():
    src = [{"id": 2, "timestamp": 2}, {"id": 1, "timestamp": 1}]
    chronological(src)
    assert [e["id"] for e in src] == [2, 1]
