"""Your agent remembers the *reason*, not just the fact — across sessions.

Real, offline, deterministic: no API key, no model download, no network. Run:

    pip install velesdb        # or: cd crates/velesdb-python && maturin develop
    python examples/agent_memory/why_across_sessions.py

An assistant stores a few things on Monday and the process exits. Weeks later a
*brand-new* session reopens the same on-disk memory and is asked "why?". Plain
vector recall finds the booking but is blind to the reason — it shares no words
with the question. why() seeds on the booking and walks the typed links to the
reason, across the session boundary. That connected context is the product.
"""

import tempfile

from velesdb import MemoryService

STORE = tempfile.mkdtemp()  # a real on-disk store; it survives process restarts


def monday_session() -> None:
    """Session 1 — the assistant learns a few things, then the process exits."""
    mem = MemoryService(STORE)
    # Build the trail so the fact that answers the question (the booking) links
    # *out* to its reason, which links on to who it's about: booking -> reason -> who.
    who = mem.remember("Robert is my father")
    reason = mem.remember(
        "He is recovering from knee surgery and needs to stretch his leg",
        links=[(who, "about")],
    )
    mem.remember("Booked the aisle seat on Robert's flight", links=[(reason, "because")])
    # the process ends here: `mem` is dropped, everything now lives on disk.


def weeks_later_session() -> None:
    """Session 2 — a fresh process reopens the SAME store and is asked why."""
    mem = MemoryService(STORE)  # reopen from disk; nothing was kept in memory
    question = "why the aisle seat on Robert's flight?"

    print(f"  recall({question!r})   — vector similarity only")
    for hit in mem.recall(question, k=2):
        print(f"     {hit['score']:.2f}  {hit['content']}")
    seen = {hit["content"] for hit in mem.recall(question, k=2)}
    assert not any("knee surgery" in c for c in seen), (
        "the reason shares no words with the question, so recall is blind to it"
    )
    print('     └─ the reason is missing: "knee surgery" shares no words with the question.\n')

    print(f"  why({question!r})      — vector seed + graph of typed links")
    nodes = mem.why(question, max_hops=2)["nodes"]
    for node in nodes:
        print(f"     hop {node['hop']}  {node['content']}")
    reached = {node["content"] for node in nodes}
    assert any("knee surgery" in c for c in reached), "why() must reach the reason via the graph"
    print("     └─ why() walked booking → reason → who, across the session boundary.")


if __name__ == "__main__":
    print("── Monday: the assistant remembers a few things, then quits ──\n")
    monday_session()
    print("── Weeks later, a NEW session reopens the same memory ──\n")
    weeks_later_session()
