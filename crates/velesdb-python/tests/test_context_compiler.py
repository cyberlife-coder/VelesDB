"""Tests for the context compiler parity on `MemoryService` (EPIC-P-071/US-001).

Exercises `compile_context` / `retrieve_context_source` / `context_savings` /
`save_working_context` / `load_working_context` — thin bindings over the exact
same `velesdb_memory::context` bridge the MCP server and the Node binding use
(zero new logic here, see `crates/velesdb-python/src/agent_memory_service.rs`).

The offline `hash` embedder keeps these deterministic and network-free, same
convention as `test_memory_service.py`.
"""

import pytest
from velesdb import MemoryService


@pytest.fixture()
def mem(tmp_path):
    return MemoryService(str(tmp_path / "store"))


# --- compile_context: minimal round trip ------------------------------------


def test_compile_context_minimal_round_trip(mem):
    req = {
        "query": "deploy pipeline",
        "token_budget": 10_000,
        "fragments": [{"content": "Never restart the primary during a rebalance."}],
    }
    out = mem.compile_context(req)
    assert "Never restart the primary during a rebalance." in out["content"]
    assert len(out["decisions"]) == 1
    assert out["decisions"][0]["action"] in {
        "preserve",
        "abstract",
        "retrieve",
        "drop",
        "cache",
    }
    assert isinstance(out["insights"], dict)


# --- compile_context: verbatim / cache fragment metadata --------------------


def test_compile_context_verbatim_and_cache_fragments(mem):
    req = {
        "query": "q",
        "token_budget": 10_000,
        "fragments": [
            {"content": "critical constraint text", "metadata": {"verbatim": True}},
            {"content": "stable prefix text", "metadata": {"cache": True}},
        ],
    }
    out = mem.compile_context(req)
    actions = {d["action"] for d in out["decisions"]}
    assert "preserve" in actions
    assert "cache" in actions


# --- retrieve_context_source: round trip + unknown handle -------------------


def test_retrieve_context_source_round_trips_a_compiled_fragment(mem):
    content_text = "Never restart the primary during a rebalance."
    req = {"query": "q", "token_budget": 10_000, "fragments": [{"content": content_text}]}
    out = mem.compile_context(req)
    handle = out["sources"][0]["handle"]
    assert handle.startswith("ctx://source/")
    source = mem.retrieve_context_source(handle)
    assert source["content"] == content_text
    assert "media" not in source or source["media"] is None


def test_retrieve_context_source_unknown_handle_raises_key_error(mem):
    with pytest.raises(KeyError):
        mem.retrieve_context_source("ctx://source/999999999999999999")


def test_retrieve_context_source_malformed_handle_raises_key_error(mem):
    with pytest.raises(KeyError):
        mem.retrieve_context_source("not-a-handle")


# --- context_savings: aggregates after a compile -----------------------------


def test_context_savings_aggregates_after_a_compile(mem):
    req = {
        "query": "deploy pipeline",
        "token_budget": 10_000,
        "project": "veles-ctx-test",
        "fragments": [
            {"content": "The deploy pipeline runs clippy before tests."},
            {"content": "The deploy pipeline runs clippy before tests."},
            {"content": "Never restart the primary during a rebalance."},
        ],
    }
    mem.compile_context(req)
    savings = mem.context_savings(project="veles-ctx-test")
    assert savings["events"] == 1
    assert savings["tokens_saved"] > 0


def test_context_savings_with_no_project_filter_returns_a_dict(mem):
    mem.compile_context(
        {"query": "q", "token_budget": 10_000, "fragments": [{"content": "a fact"}]}
    )
    savings = mem.context_savings()
    assert isinstance(savings, dict)
    assert savings["events"] >= 1


# --- save/load_working_context: round trip -----------------------------------


def test_save_and_load_working_context_round_trips(mem):
    working = {
        "goal": "ship the release",
        "active_constraints": [{"text": "never restart during rebalance"}],
        "verified_facts": [],
        "open_hypotheses": [],
        "decisions": [],
        "exact_evidence": [],
        "pending_actions": ["run smoke tests"],
    }
    wid = mem.save_working_context("veles", "session-1", working)
    assert isinstance(wid, int)

    loaded = mem.load_working_context("veles", "session-1")
    assert loaded["goal"] == "ship the release"
    assert loaded["pending_actions"] == ["run smoke tests"]
    assert loaded["active_constraints"][0]["text"] == "never restart during rebalance"


def test_save_working_context_is_an_idempotent_upsert(mem):
    first = {"goal": "first goal", "active_constraints": [], "verified_facts": [],
             "open_hypotheses": [], "decisions": [], "exact_evidence": [],
             "pending_actions": []}
    second = {"goal": "second goal", "active_constraints": [], "verified_facts": [],
              "open_hypotheses": [], "decisions": [], "exact_evidence": [],
              "pending_actions": []}
    id1 = mem.save_working_context("veles", "session-2", first)
    id2 = mem.save_working_context("veles", "session-2", second)
    assert id1 == id2
    loaded = mem.load_working_context("veles", "session-2")
    assert loaded["goal"] == "second goal"


def test_load_working_context_returns_none_when_absent(mem):
    assert mem.load_working_context("veles", "no-such-session") is None


# --- typed errors -------------------------------------------------------------


def test_compile_context_zero_budget_raises_value_error(mem):
    req = {"query": "x", "token_budget": 0, "fragments": [{"content": "y"}]}
    with pytest.raises(ValueError):
        mem.compile_context(req)


# --- wire parity: same JSON shape as the MCP `compile_context`/`context_savings`
# tools (crates/velesdb-memory/src/mcp/context_tools.rs), whose shape is itself
# exercised end-to-end by
# crates/velesdb-memory/examples/context_savings/real_measures/mcp_e2e.py.
# The single source of truth for field names is
# crates/velesdb-memory/src/context/model.rs (CompiledContext, ContextSavings).
# Documented, tolerated difference from the Node binding: ids cross as native
# Python ints (unlimited precision) here, vs decimal strings there — both are
# faithful renderings of the same u64, never truncated (see the precision test
# below).
# ------------------------------------------------------------------------------


def test_compile_context_result_shape_matches_the_mcp_wire_contract(mem):
    req = {
        "query": "deploy pipeline",
        "token_budget": 10_000,
        "project": "veles",
        "fragments": [
            {"content": "The deploy pipeline runs clippy before tests."},
            {"content": "The deploy pipeline runs clippy before tests."},
            {"content": "Never restart the primary during a rebalance."},
        ],
    }
    out = mem.compile_context(req)
    assert set(out) == {
        "content",
        "sections",
        "decisions",
        "sources",
        "retrieval_handles",
        "insights",
        "risk",
    }
    assert len(out["decisions"]) == 3
    drop = next(d for d in out["decisions"] if d["action"] == "drop")
    assert drop["rule_id"] == "drop.duplicate"
    assert isinstance(drop["fragment_id"], int)
    assert isinstance(drop["content_hash"], int)
    assert out["insights"]["tokens_saved"] > 0
    handle = out["sources"][0]["handle"]
    assert handle.startswith("ctx://source/")

    savings = mem.context_savings(project="veles")
    assert set(savings) == {
        "events",
        "tokens_in",
        "tokens_out",
        "tokens_saved",
        "cost_saved_micros_by_currency",
        "truncated",
    }
    assert savings["events"] == 1


# --- numeric boundary round-trips through the shared converters ---------------
# The u64 fix in src/utils.rs changes the whole binding's observable behavior
# (payloads, search, VelesQL, graph props all share python_to_json /
# json_to_python): ints in (i64::MAX, u64::MAX] now round-trip exactly as int
# instead of silently degrading to a lossy float. These pin the boundary
# contract: exact ints inside [i64::MIN, u64::MAX], floats stay float, bools
# stay bool (bool is an int subclass in Python — order of checks matters), and
# anything outside the range is an explicit ValueError, never silent precision
# loss.
# ------------------------------------------------------------------------------


@pytest.mark.parametrize(
    "value",
    [
        pytest.param(2**64 - 1, id="u64-max"),
        pytest.param(2**63, id="i64-max-plus-1"),
        pytest.param(1.5, id="float-stays-float"),
        pytest.param(True, id="bool-stays-bool-not-int"),
    ],
)
def test_metadata_boundary_values_round_trip_exactly(mem, value):
    fid = mem.remember("numeric boundary probe", metadata={"v": value})
    hits = mem.recall("numeric boundary probe", k=5)
    hit = next(h for h in hits if h["id"] == fid)
    got = hit["metadata"]["v"]
    assert got == value
    # `True == 1` and `1 == 1.0` in Python, so equality alone cannot catch a
    # bool→int or int→float degradation — the exact type must survive too.
    assert type(got) is type(value)


def test_metadata_int_above_u64_max_raises_value_error(mem):
    # 2**64 fits neither i64 nor u64; before the fix it silently fell through
    # to a lossy f64 (1.8446744073709552e19) — it must be an explicit error.
    with pytest.raises(ValueError):
        mem.remember("too big", metadata={"v": 2**64})


def test_metadata_int_below_i64_min_raises_value_error(mem):
    # Same guard on the low side: -(2**63) - 1 is below i64::MIN.
    with pytest.raises(ValueError):
        mem.remember("too small", metadata={"v": -(2**63) - 1})


def test_compile_context_ids_survive_full_u64_precision(mem):
    # stable_id (FNV-1a 64) ids are uniform over u64, so ~50% exceed
    # i64::MAX; the binding must carry them as native Python ints, never a
    # lossy float (this is the "unlimited precision" id contract).
    big_id = 18_446_744_073_709_551_615  # u64::MAX
    req = {
        "query": "large id probe",
        "token_budget": 10_000,
        "fragments": [{"content": "x" * 40, "id": big_id}],
    }
    out = mem.compile_context(req)
    decision = out["decisions"][0]
    assert decision["fragment_id"] == big_id
