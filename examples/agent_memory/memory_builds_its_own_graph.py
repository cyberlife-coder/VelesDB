"""You didn't draw a single edge. Paste a paragraph — the memory builds the graph.

Local model, runs on your machine, nothing leaves it:

    cd crates/velesdb-python && maturin develop --features extract   # build with extract
    ollama pull qwen3.6:27b-mlx && ollama pull all-minilm
    python examples/agent_memory/memory_builds_its_own_graph.py

remember_extracted() reads raw prose with a local LLM, stores the atomic facts,
and *auto-wires* the fact↔topic graph. There is no relate() call and no links=
anywhere below — the connections are discovered, not hand-built. Then why() walks
that self-built graph from a surface question down to the real root cause, while
plain recall returns only the facts that look like the question.
"""

import tempfile

from velesdb import MemoryService

EXTRACT_MODEL = "qwen3.6:27b-mlx"

# One block of raw notes — exactly what an agent would capture from a meeting.
TRANSCRIPT = """Engineering standup, March 3rd.
We moved the analytics export to run at 3am instead of midnight.
The export kept colliding with the database backup, and that collision caused the
big Sunday slowdown two weeks ago. The backup itself cannot be moved, because our
storage vendor only gives us a maintenance window at midnight. Priya now owns the
analytics export.
"""


def facts_only(nodes):
    """Drop the topic-hub nodes ('Entity: …') so we show the fact chain only."""
    return [n for n in nodes if not n["content"].startswith("Entity:")]


def main() -> None:
    store = tempfile.mkdtemp()
    mem = MemoryService(store, embedder="ollama", ollama_model="all-minilm")

    print("── Raw notes handed to the agent (no structure, no links) ──\n")
    print("   " + TRANSCRIPT.strip().replace("\n", "\n   ") + "\n")

    print("── remember_extracted(): a local model extracts facts + builds the graph ──\n")
    ids = mem.remember_extracted(TRANSCRIPT, model=EXTRACT_MODEL)
    print(f"   stored {len(ids)} atomic facts — and zero edges were written by hand.\n")

    del mem
    mem = MemoryService(store, embedder="ollama", ollama_model="all-minilm")
    question = "why did the analytics export move to 3am?"

    print(f"  recall({question!r})   — vector similarity")
    for hit in mem.recall(question, k=2):
        print(f"     {hit['score']:.2f}  {hit['content']}")
    print("     └─ it returns the move itself — the surface answer.\n")

    print(f"  why({question!r})      — walks the self-built graph to the root cause")
    for node in facts_only(mem.why(question, max_hops=4)["nodes"]):
        print(f"     hop {node['hop']}  {node['content']}")
    print("     └─ from a one-line question down to the storage vendor's")
    print("        midnight window — a graph nobody drew by hand.")


if __name__ == "__main__":
    main()
