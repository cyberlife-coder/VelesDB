from velesdb_common.memory import format_procedural_results


def test_format_procedural_results_basic():
    results = [
        {"name": "proc1", "steps": ["a", "b"], "confidence": 0.9, "score": 0.85},
    ]
    formatted = format_procedural_results(results)
    assert len(formatted) == 1
    assert formatted[0]["name"] == "proc1"
    assert formatted[0]["steps"] == ["a", "b"]
    assert formatted[0]["confidence"] == 0.9
    assert formatted[0]["score"] == 0.85


def test_format_procedural_results_empty():
    assert format_procedural_results([]) == []
