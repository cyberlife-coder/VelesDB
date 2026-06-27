"""The wedge in 60 seconds — what `why()` does that plain recall can't.

Offline, deterministic, no API key. Run:

    cd crates/velesdb-python && maturin develop && cd -   # or: pip install velesdb
    python examples/agent_memory/wedge_quickstart.py

It stores a small decision trail (decision -> PR -> ticket) wired with typed
links, then asks the same question two ways:

* `recall()` ranks by vector similarity and is blind to the ticket, which shares
  no words with the question.
* `why()` seeds on the closest memory and walks the graph, reaching the very
  ticket the decision fixed.

That gap — connected context a similarity search misses — is the product.
"""

import tempfile

from velesdb import MemoryService


def main() -> None:
    # "hash" embedder: deterministic, offline, no model download.
    mem = MemoryService(tempfile.mkdtemp())

    # A decision trail wired with typed links: the decision was made in a PR,
    # which fixes the original ticket.
    ticket = mem.remember("EPIC-317: intermittent CI hang under load")
    pr = mem.remember(
        "PR #42 swaps the std Mutex for parking_lot",
        links=[(ticket, "fixes")],
    )
    mem.remember(
        "we chose parking_lot to avoid lock poisoning after a panic",
        links=[(pr, "decided_in")],
    )

    question = "why did we choose parking_lot"

    print(f"recall({question!r})   [vector similarity only]")
    for h in mem.recall(question, k=2):
        print(f"   {h['score']:.2f}  {h['content']}")
    recalled = {h["content"] for h in mem.recall(question, k=2)}
    assert not any("EPIC-317" in c for c in recalled), (
        "the ticket shares no words with the question, so recall is blind to it"
    )
    print("   └─ EPIC-317 is nowhere here: it shares no words with the question.\n")

    print(f"why({question!r})      [vector seed + graph traversal]")
    explanation = mem.why(question, max_hops=2)
    for node in explanation["nodes"]:
        print(f"   hop {node['hop']}  {node['content']}")
    reached = {node["content"] for node in explanation["nodes"]}
    assert any("EPIC-317" in c for c in reached), "why() must reach the ticket via the graph"
    print("   └─ the graph reached the very ticket the decision fixed. That gap is the product.")


if __name__ == "__main__":
    main()
