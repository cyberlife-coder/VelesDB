"""Tests for the high-level MemoryService wedge (remember/recall/relate/forget/why).

These exercise the Python binding over the same hardened Rust the MCP server uses.
The offline `hash` embedder keeps them deterministic and network-free.
"""

import json
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest
from velesdb import MemoryService


class _EmbeddingHandler(BaseHTTPRequestHandler):
    """Minimal Ollama-compatible probe endpoint for constructor tests."""

    def do_POST(self):  # noqa: N802 - BaseHTTPRequestHandler API
        length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(length)
        payload = json.dumps({"embedding": [0.1, 0.2, 0.3, 0.4]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, _format, *_args):
        """Keep the fixture silent so only binding stderr is asserted."""


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


# --- entity -----------------------------------------------------------------
# The read path for questions ABOUT a named thing. Entity hubs are only ever
# created by extraction, which used to mean a running Ollama — so this file
# could exercise the MISS contract and nothing else, and the hit shape went
# unchecked for as long as that held. The `"outline"` extractor took the
# network out of that path, so both branches are covered below.


def test_entity_unknown_name_reports_a_miss_and_echoes_the_canonical_query(mem):
    profile = mem.entity("  Alex Martin  ")
    assert profile["found"] is False
    assert profile["id"] == 0
    # A miss carries no name of its own: the query is echoed canonicalized so
    # several lookups can be paired with their question.
    assert profile["name"] == "alex martin"
    assert profile["attributes"] == {}
    # BOTH edge lists are present on a miss, empty. A shape that changed with
    # the outcome would force every caller to branch, and the binding parity
    # guard cannot see it — it reads a declaration, never an execution path.
    assert profile["relations"] == []
    assert profile["relations_in"] == []


def test_entity_does_not_surface_a_mentioning_sentence(mem):
    # A remembered sentence mentioning a name creates NO entity hub — that is
    # the whole distinction between recall (sentences) and entity (things).
    mem.remember("Alex Martin shipped the parking_lot migration")
    assert mem.recall("Alex Martin", k=5), "recall does find the sentence"
    assert mem.entity("Alex Martin")["found"] is False


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


def test_oversized_metadata_raises_value_error(mem):
    # metadata is capped at 64 KiB serialized (a DoS guard: metadata is a
    # keyed lookup facet, not a payload) — a caller-supplied blob past the
    # cap must raise, not silently persist.
    with pytest.raises(ValueError):
        mem.remember("x", metadata={"v": "y" * (65 * 1024)})


def test_unknown_link_target_raises_key_error(mem):
    with pytest.raises(KeyError):
        mem.remember("a decision", links=[(9_999_999, "decided_in")])


def test_unknown_embedder_raises_value_error():
    with pytest.raises(ValueError):
        MemoryService(tempfile.mkdtemp(), embedder="nope")


def test_open_with_hash_warns_once_and_names_the_semantic_argument(
    tmp_path, capfd, monkeypatch
):
    monkeypatch.delenv("VELESDB_MEMORY_QUIET", raising=False)
    MemoryService(str(tmp_path / "hash-store"), embedder="hash")

    stderr = capfd.readouterr().err
    assert stderr.count("NOT semantic") == 1
    assert 'embedder="ollama"' in stderr
    assert "VELESDB_MEMORY_QUIET=1" in stderr


def test_quiet_environment_suppresses_the_hash_notice(tmp_path, capfd, monkeypatch):
    monkeypatch.setenv("VELESDB_MEMORY_QUIET", "1")
    MemoryService(str(tmp_path / "quiet-store"), embedder="hash")

    assert "NOT semantic" not in capfd.readouterr().err


def test_open_with_ollama_emits_no_degraded_hash_notice(tmp_path, capfd, monkeypatch):
    monkeypatch.delenv("VELESDB_MEMORY_QUIET", raising=False)
    server = ThreadingHTTPServer(("127.0.0.1", 0), _EmbeddingHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = server.server_address
        memory = MemoryService(
            str(tmp_path / "ollama-store"),
            embedder="ollama",
            ollama_url=f"http://{host}:{port}",
            # The backend, not an arbitrary remote model name, decides semantics.
            ollama_model="hash",
        )
        assert memory.memory_status()["embedder"]["semantic"] is True
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)

    assert "NOT semantic" not in capfd.readouterr().err


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
    # `remember` also auto-stamps `_veles_date` (today's date, YYYYMMDD) onto
    # every fact unless the caller already set it, so the metadata dict is
    # no longer exactly `{"ts": ...}` — check the caller-supplied key plus
    # the presence of the auto date, not exact dict equality.
    fid = mem.remember("we shipped the release", metadata={"ts": 20260701})
    hits = mem.recall_where("release", [("ts", "eq", 20260701)], k=5)
    hit = next(h for h in hits if h["id"] == fid)
    assert hit["metadata"]["ts"] == 20260701
    assert isinstance(hit["metadata"]["_veles_date"], int)


def test_recall_also_returns_stored_metadata(mem):
    # `recall` round-trips caller metadata too (one extra by-id lookup per
    # hit), not just `recall_where` — enables dated/sorted context from any
    # recall path. See the `_veles_date` note above.
    fid = mem.remember("paris is lovely in spring", metadata={"ts": 1})
    hits = mem.recall("paris", k=5)
    hit = next(h for h in hits if h["id"] == fid)
    assert hit["metadata"]["ts"] == 1
    assert isinstance(hit["metadata"]["_veles_date"], int)


def test_recall_metadata_holds_only_the_auto_date_when_the_fact_carries_no_caller_metadata(
    mem,
):
    # `remember` now auto-stamps every fact with `_veles_date` (today's date,
    # a YYYYMMDD integer) unless the caller already set it, so a fact given
    # no caller metadata no longer round-trips as `metadata: None` — it
    # round-trips as `{"_veles_date": <today>}` and nothing else.
    mem.remember("a fact with no metadata")
    hits = mem.recall("a fact with no metadata", k=5)
    for h in hits:
        assert set(h["metadata"].keys()) == {"_veles_date"}
        assert isinstance(h["metadata"]["_veles_date"], int)


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


def test_unrelate_removes_the_edge_and_keeps_both_facts(mem):
    # relate's exact undo: the edge stops being traversable, the two facts stay.
    decision = mem.remember("we chose parking_lot to avoid lock poisoning")
    ticket = mem.remember("EPIC-317 xyzzy quux frobnicate")
    mem.relate(decision, ticket, "decided_in")

    def reached():
        why = mem.why("we chose parking_lot to avoid lock poisoning", max_hops=2)
        return any(n["id"] == ticket for n in why["nodes"])

    assert reached(), "precondition: the edge makes the ticket traversable"

    outcome = mem.unrelate(decision, ticket, "decided_in")
    assert outcome == {"found": True, "removed": 1}
    assert not reached(), "the edge is really gone"

    survivors = {h["id"] for h in mem.recall("parking_lot lock poisoning EPIC-317", k=5)}
    assert decision in survivors and ticket in survivors


def test_unrelate_is_idempotent_on_an_absent_edge(mem):
    # An absent edge is an answer, never an exception — a cleanup must replay.
    a = mem.remember("fact a for the unrelate guard")
    b = mem.remember("fact b for the unrelate guard")
    assert mem.unrelate(a, b, "decided_in") == {"found": False, "removed": 0}

    mem.relate(a, b, "decided_in")
    assert mem.unrelate(a, b, "decided_in") == {"found": True, "removed": 1}
    assert mem.unrelate(a, b, "decided_in") == {"found": False, "removed": 0}


def test_unrelate_refuses_exactly_what_relate_refuses(mem):
    # Same taxonomy as relate: invalid input surfaces as ValueError.
    a = mem.remember("fact a for the unrelate refusals")
    b = mem.remember("fact b for the unrelate refusals")
    with pytest.raises(ValueError):
        mem.unrelate(a, b, "")
    with pytest.raises(ValueError):
        mem.unrelate(a, a, "decided_in")


# --- remember_extracted, and the entity HIT it makes reachable ---------------
# Before the `"outline"` backend these had no offline proof at all: the only
# Extractor in the crate called a generative model over the network, so both
# `skipped_over_cap` and the incoming half of a profile were declared KNOWN
# GAPs in the binding parity guard rather than tested (issues #1690, #1692).


def test_outlined_edge_reaches_the_far_end_as_an_incoming_relation(mem):
    mem.remember_extracted(
        "edge: Camille | sister of | Theo",
        extractor="outline",
    )
    theo = mem.entity("Theo")
    assert theo["found"] is True
    # The edge LEAVES camille, so it is invisible from theo's outgoing list
    # and reachable only here. A binding relaying `relations_in` by copying
    # `relations` would pass the miss test above and fail this one.
    assert [r["predicate"] for r in theo["relations_in"]] == ["sister of"]
    assert theo["relations"] == []

    camille = mem.entity("Camille")
    assert [r["predicate"] for r in camille["relations"]] == ["sister of"]
    assert camille["relations_in"] == []


def test_remember_extracted_reports_what_it_dropped(mem):
    outcome = mem.remember_extracted(
        "fact: Camille ships the parser. | camille\n"
        "fact: " + "x" * 4096 + "\n"
        "edge: Camille | works at | Wiscale",
        extractor="outline",
    )
    # An envelope, not a bare list: a shorter list of ids cannot say whether
    # the passage held fewer facts or lost some to the embeddable cap.
    assert outcome["ids"] and len(outcome["ids"]) == 1
    assert outcome["skipped_over_cap"] == 1


def test_remember_extracted_keeps_an_attribute_json_type(mem):
    mem.remember_extracted("attr: Theo Durand | age | 15", extractor="outline")
    # `recall_where` comparisons are type-strict, so an age arriving as the
    # string "15" would silently never match a numeric filter.
    assert mem.entity("Theo Durand")["attributes"]["age"] == 15


def test_unknown_extractor_raises_value_error(mem):
    # Mirrors test_unknown_embedder_raises_value_error: a caller who asked for
    # a backend that does not exist is told so, never handed another one.
    with pytest.raises(ValueError, match="unknown extractor"):
        mem.remember_extracted("fact: anything", extractor="nope")


def test_ollama_extractor_without_a_model_raises_value_error(mem):
    # `model` became optional when `extractor` landed; the ollama backend
    # still needs one, and says which flag to reach for instead.
    with pytest.raises(ValueError, match="needs a model"):
        mem.remember_extracted("fact: anything")


def test_a_malformed_outline_directive_refuses_instead_of_dropping_the_line(mem):
    # A graph that quietly loses half of what it was handed is worse than one
    # that refuses.
    with pytest.raises(Exception, match=r"3 `\|`-separated fields, 2 given"):
        mem.remember_extracted("edge: Camille | works at", extractor="outline")


def test_memory_status_reports_the_hash_default_as_not_semantic(mem):
    """The binding's status mirrors the MCP envelope, with the binding's
    own truths: the constructor resolved the embedder, nothing is
    pre-attached for extraction, and the counts are live."""
    before = mem.memory_status()
    assert before["embedder"]["model"] == "hash"
    assert before["embedder"]["semantic"] is False
    assert before["provenance"]["recorded"] is False
    assert before["extraction"]["configured"] is False
    assert before["memory"]["facts"] == 0
    assert before["memory"]["edges"] == 0

    a = mem.remember("le port est 6333")
    b = mem.remember("l'incident est INC-42")
    mem.relate(a, b, "explique")

    after = mem.memory_status()
    assert after["memory"]["facts"] == 2
    assert after["memory"]["edges"] >= 1


def test_list_memories_walks_the_store_exhaustively(mem):
    """Cursor pagination sees every fact exactly once, ids ascending, and a
    metadata filter narrows without erroring on a miss."""
    for i in range(5):
        mem.remember(f"fait numero {i}", metadata={"project": "acme"})

    seen = []
    cursor = None
    for _ in range(16):
        page = mem.list_memories(cursor=cursor, limit=2)
        seen.extend(page["memories"])
        if page["next_cursor"] is None:
            break
        cursor = int(page["next_cursor"])
    assert len(seen) == 5
    ids = [entry["id"] for entry in seen]
    assert ids == sorted(ids), "ids come back ascending"
    assert all(entry["metadata"]["project"] == "acme" for entry in seen)

    filtered = mem.list_memories(filter={"project": "globex"})
    assert filtered["memories"] == []
