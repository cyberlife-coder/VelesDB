"""Offline tests for the LangGraph memory tools — no LLM, deterministic."""

import tempfile

from langgraph_velesdb import make_memory_tools


def _tools_by_name():
    return {t.name: t for t in make_memory_tools(tempfile.mkdtemp())}


def test_exposes_the_four_memory_tools():
    assert set(_tools_by_name()) == {"remember", "recall", "relate", "why"}


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
