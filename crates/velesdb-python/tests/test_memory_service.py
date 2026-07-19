"""Tests for the high-level MemoryService wedge (remember/recall/relate/forget/why).

These exercise the Python binding over the same hardened Rust the MCP server uses.
The offline `hash` embedder keeps them deterministic and network-free.
"""

import tempfile

import pytest
from velesdb import MemoryService


@pytest.fixture()
def mem(tmp_path):
    return MemoryService(str(tmp_path / "store"))


def test_remember_returns_stable_id(mem):
    a = mem.remember("Paris is the capital of France")
    b = mem.remember("Paris is the capital of France")
    assert a == b  # content-addressed → idempotent


def test_recall_finds_a_stored_fact(mem):
    fid = mem.remember("we chose parking_lot to avoid lock poisoning")
    hits = mem.recall("parking_lot poisoning", k=5)
    assert any(h["id"] == fid for h in hits)
    assert all({"id", "score", "content"} <= set(h) for h in hits)


def test_recall_filter_narrows_to_metadata(mem):
    keep = mem.remember("auth bug in login", metadata={"project": "veles"})
    mem.remember("auth bug in login too", metadata={"project": "acme"})
    hits = mem.recall("auth bug", k=5, filter={"project": "veles"})
    ids = {h["id"] for h in hits}
    assert keep in ids
    assert all(h["id"] == keep for h in hits)


def test_why_returns_the_connected_subgraph(mem):
    pr = mem.remember("PR #42 swaps the std Mutex for parking_lot")
    dec = mem.remember(
        "we chose parking_lot to avoid lock poisoning",
        links=[(pr, "decided_in")],
    )
    why = mem.why("why did we choose parking_lot", max_hops=2)
    node_ids = [n["id"] for n in why["nodes"]]
    assert dec in node_ids and pr in node_ids
    assert any(e["relation"] == "decided_in" for e in why["edges"])


def test_forget_removes_a_memory(mem):
    fid = mem.remember("ephemeral note about France")
    assert mem.forget(fid) is True
    hits = mem.recall("France", k=5)
    assert all(h["id"] != fid for h in hits)


def test_forget_unknown_id_reports_not_found(mem):
    # An id that was never stored: a no-op, not an error — but the caller
    # must be able to tell it apart from a real deletion.
    assert mem.forget(999_999) is False


def test_reserved_metadata_key_raises_value_error(mem):
    with pytest.raises(ValueError):
        mem.remember("x", metadata={"_veles_hub": True})


def test_unknown_link_target_raises_key_error(mem):
    with pytest.raises(KeyError):
        mem.remember("a decision", links=[(9_999_999, "decided_in")])


def test_unknown_embedder_raises_value_error():
    with pytest.raises(ValueError):
        MemoryService(tempfile.mkdtemp(), embedder="nope")


def test_why_huge_max_hops_is_silently_capped(mem):
    # The binding caps max_hops at 10 (same as the MCP server) to prevent
    # unbounded graph traversal; passing usize::MAX must not hang or error.
    fid = mem.remember("rust is a systems language")
    why = mem.why("rust", max_hops=10_000)
    node_ids = [n["id"] for n in why["nodes"]]
    assert fid in node_ids


def test_recall_where_eq_matches_metadata_filter(mem):
    # An `eq` column filter equals the exact-match recall filter (same engine).
    keep = mem.remember("auth bug in login", metadata={"project": "veles"})
    mem.remember("auth bug elsewhere", metadata={"project": "acme"})
    hits = mem.recall_where("auth bug", [("project", "eq", "veles")], k=5)
    ids = {h["id"] for h in hits}
    assert keep in ids
    assert all(h["id"] == keep for h in hits)


def test_recall_where_numeric_range_filters(mem):
    # A year range a vector store cannot express; the ColumnStore predicate can.
    inrange = mem.remember("alice was CEO in 2003", metadata={"year": 2003})
    mem.remember("bob was CEO in 2010", metadata={"year": 2010})
    hits = mem.recall_where(
        "who was CEO", [("year", "ge", 2000), ("year", "le", 2005)], k=5
    )
    ids = {h["id"] for h in hits}
    assert inrange in ids
    assert all(h["id"] == inrange for h in hits)


def test_recall_where_unknown_op_raises_value_error(mem):
    with pytest.raises(ValueError):
        mem.recall_where("q", [("year", "bogus", 1)], k=5)


def test_recall_where_returns_stored_metadata(mem):
    # `recall_where` results carry the fact's caller-supplied metadata dict.
    fid = mem.remember("we shipped the release", metadata={"ts": 20260701})
    hits = mem.recall_where("release", [("ts", "eq", 20260701)], k=5)
    hit = next(h for h in hits if h["id"] == fid)
    assert hit["metadata"] == {"ts": 20260701}


def test_recall_also_returns_stored_metadata(mem):
    # `recall` round-trips caller metadata too (one extra by-id lookup per
    # hit), not just `recall_where` — enables dated/sorted context from any
    # recall path.
    fid = mem.remember("paris is lovely in spring", metadata={"ts": 1})
    hits = mem.recall("paris", k=5)
    hit = next(h for h in hits if h["id"] == fid)
    assert hit["metadata"] == {"ts": 1}


def test_recall_metadata_is_none_when_the_fact_carries_none(mem):
    mem.remember("a fact with no metadata")
    hits = mem.recall("a fact with no metadata", k=5)
    assert all(h["metadata"] is None for h in hits)


def test_recall_fused_folds_in_a_graph_reached_fact(mem):
    # Fused recall walks the graph from the top vector hit and folds in a fact
    # the query never mentions but a stored link connects — the shipped
    # tri-engine ranking, not a harness-only prompt trick.
    anchor = mem.remember("we chose parking_lot to avoid lock poisoning")
    linked = mem.remember(
        "the on-call rotation moved to Tuesdays",
        links=[(anchor, "context")],
    )
    # Plain top-1 vector recall finds the anchor, not the unrelated linked fact.
    plain = mem.recall("parking_lot poisoning", k=1)
    assert all(h["id"] != linked for h in plain)
    # Fused recall reaches it through the graph.
    fused = mem.recall_fused("parking_lot poisoning", k=10)
    ids = {h["id"] for h in fused}
    assert anchor in ids and linked in ids


def test_recall_fused_respects_exact_match_filter(mem):
    keep = mem.remember("auth bug in login", metadata={"project": "veles"})
    mem.remember("auth bug in login too", metadata={"project": "acme"})
    hits = mem.recall_fused("auth bug", k=5, filter={"project": "veles"})
    assert all(h["id"] == keep for h in hits)


def test_recall_fused_accepts_tuning_knobs(mem):
    # Advanced fusion knobs go in `options` (same shape as Node/WASM); optional
    # and clamped, not rejected.
    mem.remember("a decision about locks")
    hits = mem.recall_fused("locks", k=5, options={"hops": 1, "graph_boost": 0.3, "pool": 64})
    assert isinstance(hits, list)


def test_recall_fused_survives_non_finite_graph_boost(mem):
    # A native Python float bypasses JSON's NaN rejection, so the binding must
    # not let a NaN graph_boost poison fusion (it would collapse the ranking and
    # silently drop exactly the graph-reached facts recall_fused exists to find).
    anchor = mem.remember("we chose parking_lot to avoid lock poisoning")
    linked = mem.remember(
        "the on-call rotation moved to Tuesdays",
        links=[(anchor, "context")],
    )
    hits = mem.recall_fused(
        "parking_lot poisoning", k=10, options={"graph_boost": float("nan")}
    )
    ids = {h["id"] for h in hits}
    assert linked in ids


def test_recall_fused_dated_returns_timeline_and_now(mem):
    # With date_field, recall_fused returns a dict carrying a chronological,
    # date-prefixed timeline + a "now" anchor — the temporal representation
    # shipped as product behavior, not left to the caller's prompt.
    mem.remember("the release shipped", metadata={"ts": 20260701})
    mem.remember("the project kicked off", metadata={"ts": 20260103})
    res = mem.recall_fused("project release timeline", k=10, date_field="ts")
    assert isinstance(res, dict)
    assert set(res) == {"memories", "dated_context", "now"}
    timeline = res["dated_context"]
    assert "- [2026-01-03] the project kicked off" in timeline
    assert "- [2026-07-01] the release shipped" in timeline
    # Oldest first.
    assert timeline.index("2026-01-03") < timeline.index("2026-07-01")
    assert res["now"] == "2026-07-01"


def test_recall_fused_without_date_field_returns_a_plain_list(mem):
    # Backward-compatible: no date_field -> a list, exactly like before.
    mem.remember("a plain fact")
    res = mem.recall_fused("plain fact", k=5)
    assert isinstance(res, list)


def test_recall_fused_zero_pool_is_floored_not_emptied(mem):
    # pool=0 must not oversample zero candidates and return nothing; it is
    # floored to 1 (a deliberate small pool is still honored, just never empty).
    for i in range(3):
        mem.remember(f"a fact number {i} about locks")
    hits = mem.recall_fused("locks", k=5, options={"pool": 0})
    assert len(hits) > 0


def test_oversized_fact_raises_value_error(mem):
    # Facts above the shared 1 MiB cap are rejected before any embedding work.
    with pytest.raises(ValueError):
        mem.remember("x" * (1024 * 1024 + 1))


def test_feedback_success_increases_confidence_and_roundtrips(mem):
    # remember -> feedback(id, True) returns a float, and repeated positive
    # feedback moves confidence monotonically upward (the RL loop learning).
    fid = mem.remember("we chose parking_lot to avoid lock poisoning")
    first = mem.feedback(fid, True)
    assert isinstance(first, float)
    second = mem.feedback(fid, True)
    assert second > first


def test_feedback_unknown_id_raises_key_error(mem):
    # Same taxonomy as forget: a missing memory id is a KeyError, not a
    # silent no-op — feedback has no result to report if the fact is gone.
    with pytest.raises(KeyError):
        mem.feedback(999_999, True)
