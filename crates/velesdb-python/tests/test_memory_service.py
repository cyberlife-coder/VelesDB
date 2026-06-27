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
    mem.forget(fid)
    hits = mem.recall("France", k=5)
    assert all(h["id"] != fid for h in hits)


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
