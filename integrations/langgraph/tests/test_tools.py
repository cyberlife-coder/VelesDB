"""Offline tests for the LangGraph memory tools — no LLM, deterministic."""

import tempfile

from langgraph_velesdb import make_memory_tools


def _tools_by_name():
    return {t.name: t for t in make_memory_tools(tempfile.mkdtemp())}


class _Pre0_11Service:
    """Stands in for a ``MemoryService`` built against a velesdb wheel
    published before ``feedback``/``save_working_context``/
    ``load_working_context`` existed (e.g. the current PyPI 3.12.0) — same
    shape as the real binding, minus those three methods.
    """

    def __init__(self):
        self._facts = {}
        self._next_id = 1

    def remember(self, fact, links=None, metadata=None, ttl_seconds=None):
        fid = self._next_id
        self._next_id += 1
        self._facts[fid] = fact
        return fid

    def recall(self, query, k):
        return [{"id": i, "score": 1.0, "content": f, "metadata": None} for i, f in self._facts.items()]

    def recall_where(self, query, filters, k):
        return self.recall(query, k)

    def recall_fused(self, query, k, filter=None, *, date_field=None, options=None):
        return self.recall(query, k)

    def relate(self, from_id, to_id, relation):
        return 1

    def forget(self, id):
        return self._facts.pop(id, None) is not None

    def why(self, question, max_hops):
        return {"nodes": [], "edges": []}

    # feedback / save_working_context / load_working_context intentionally
    # absent, matching the real gap on PyPI 3.12.0.


def test_exposes_the_full_memory_tool_set():
    assert set(_tools_by_name()) == {
        "remember",
        "recall",
        "recall_where",
        "recall_fused",
        "relate",
        "forget",
        "feedback",
        "why",
        "save_working_context",
        "load_working_context",
    }


def test_why_reaches_connected_context_that_recall_misses():
    tools = _tools_by_name()

    # The booking is the only fact that shares words with the question, so it is
    # recall's clear top hit; the reason shares none and stays out of the top-1.
    reason = tools["remember"].invoke(
        {"fact": "He is recovering from knee surgery and needs to stretch his leg"}
    )
    booking = tools["remember"].invoke(
        {"fact": "Booked the aisle seat on Robert's flight"}
    )
    tools["relate"].invoke({"from_id": booking, "to_id": reason, "relation": "because"})

    question = "why the aisle seat on Robert's flight?"
    recalled = {hit["content"] for hit in tools["recall"].invoke({"query": question, "k": 1})}
    assert not any("knee surgery" in c for c in recalled)

    reached = {node["content"] for node in tools["why"].invoke({"question": question})["nodes"]}
    assert any("knee surgery" in c for c in reached)


def test_requires_path_or_service():
    try:
        make_memory_tools()
    except ValueError:
        return
    raise AssertionError("expected ValueError when neither path nor service is given")


def test_forget_removes_a_memory_and_reports_whether_it_existed():
    tools = _tools_by_name()

    fid = tools["remember"].invoke({"fact": "The deploy key rotates every 90 days"})

    assert tools["forget"].invoke({"id": fid}) is True
    # Second delete of the same id is a no-op, not an error.
    assert tools["forget"].invoke({"id": fid}) is False


def test_feedback_moves_confidence_after_use():
    tools = _tools_by_name()

    fid = tools["remember"].invoke({"fact": "Staging uses the blue database"})

    reinforced = tools["feedback"].invoke({"id": fid, "success": True})
    weakened = tools["feedback"].invoke({"id": fid, "success": False})

    assert 0.0 <= weakened < reinforced <= 1.0


def test_remember_accepts_metadata_links_and_ttl_without_changing_defaults():
    tools = _tools_by_name()

    # Bare call keeps working exactly as before (defaults unchanged).
    plain_id = tools["remember"].invoke({"fact": "The mascot is a red panda"})
    assert isinstance(plain_id, int)

    pr = tools["remember"].invoke({"fact": "PR #42 swaps the mutex for parking_lot"})
    decision = tools["remember"].invoke(
        {
            "fact": "We chose parking_lot to avoid lock poisoning",
            "links": [[pr, "decided_in"]],
            "metadata": {"topic": "concurrency"},
            "ttl_seconds": 3600,
        }
    )

    hits = tools["recall_where"].invoke(
        {"query": "parking_lot", "filters": [["topic", "eq", "concurrency"]]}
    )
    assert any(h["id"] == decision for h in hits)


def test_recall_where_surfaces_metadata_including_the_auto_date_stamp():
    tools = _tools_by_name()

    tools["remember"].invoke({"fact": "Rotated the signing key"})

    hits = tools["recall_where"].invoke({"query": "signing key", "filters": []})

    assert hits
    assert "_veles_date" in hits[0]["metadata"]


def test_recall_fused_with_date_field_returns_a_dated_timeline():
    tools = _tools_by_name()

    tools["remember"].invoke({"fact": "Rotated the signing key today"})

    result = tools["recall_fused"].invoke(
        {"query": "signing key", "date_field": "_veles_date"}
    )

    assert "dated_context" in result
    assert "memories" in result


def test_recall_fused_without_date_field_returns_a_plain_list():
    tools = _tools_by_name()

    tools["remember"].invoke({"fact": "Rotated the signing key today"})

    result = tools["recall_fused"].invoke({"query": "signing key"})

    assert isinstance(result, list)


def test_save_and_load_working_context_round_trips():
    tools = _tools_by_name()

    working = {"goal": "ship issue 1546", "pending_actions": ["open the PR"]}
    tools["save_working_context"].invoke(
        {"project": "veles", "session": "session-1", "working": working}
    )

    loaded = tools["load_working_context"].invoke(
        {"project": "veles", "session": "session-1"}
    )
    assert loaded["goal"] == "ship issue 1546"


def test_load_working_context_returns_none_when_nothing_was_saved():
    tools = _tools_by_name()

    loaded = tools["load_working_context"].invoke(
        {"project": "veles", "session": "no-such-session"}
    )
    assert loaded is None


def _tools_on_pre_0_11_binding():
    return {t.name: t for t in make_memory_tools(service=_Pre0_11Service())}


def test_make_memory_tools_construction_succeeds_even_without_new_binding_methods():
    # The guard is a call-time check, not a construction-time one: building
    # the tool list must not itself require feedback/save_working_context/
    # load_working_context to exist on the service.
    tools = _tools_on_pre_0_11_binding()
    assert set(tools) == {
        "remember",
        "recall",
        "recall_where",
        "recall_fused",
        "relate",
        "forget",
        "feedback",
        "why",
        "save_working_context",
        "load_working_context",
    }


def test_feedback_returns_actionable_error_instead_of_raising_when_unsupported():
    tools = _tools_on_pre_0_11_binding()

    result = tools["feedback"].invoke({"id": 1, "success": True})

    assert result == {
        "error": "feedback requires velesdb > 3.12.0 — upgrade with `pip install -U velesdb`"
    }


def test_save_working_context_returns_actionable_error_instead_of_raising_when_unsupported():
    tools = _tools_on_pre_0_11_binding()

    result = tools["save_working_context"].invoke(
        {"project": "veles", "session": "s1", "working": {"goal": "x"}}
    )

    assert result == {
        "error": "save_working_context requires velesdb > 3.12.0 — upgrade with `pip install -U velesdb`"
    }


def test_load_working_context_returns_actionable_error_instead_of_raising_when_unsupported():
    tools = _tools_on_pre_0_11_binding()

    result = tools["load_working_context"].invoke({"project": "veles", "session": "s1"})

    assert result == {
        "error": "load_working_context requires velesdb > 3.12.0 — upgrade with `pip install -U velesdb`"
    }
