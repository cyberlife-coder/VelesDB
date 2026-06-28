"""Why is this magic constant here? Your agent remembers the *reason*, not just the value.

Real, offline, deterministic: no API key, no model download, no network.

    pip install velesdb        # or: cd crates/velesdb-python && maturin develop
    python examples/agent_memory/why_magic_constant.py

A teammate set a request timeout to 7 seconds months ago. You're about to "clean
it up" to a rounder number. Plain vector recall surfaces the timeout line and a
pile of look-alike config — but it is blind to *why* the value is 7: a business
reason that shares no words with the code. why() seeds on the timeout decision
and walks the typed link to that reason, so your agent warns you before you break
a customer. (Verified embedder-robust: even a real semantic model keeps recall
blind to the reason — see the README note.)
"""

import tempfile

from velesdb import MemoryService

STORE = tempfile.mkdtemp()

# A real code project's memory: one decision linked to the human reason behind it,
# plus the ordinary config facts a coding agent would have stored along the way.
PROJECT_FACTS = [
    "The HTTP server listens on port 8080 by default",
    "We use clap for CLI argument parsing",
    "Benchmarks run nightly on the self-hosted M2 runner",
    "Auth tokens expire after 24 hours",
    "The release workflow publishes to crates.io on tag push",
    "Docs are built with mdBook and deployed to GitHub Pages",
    "We pin the Rust toolchain to 1.89 in CI",
    "The vector index uses HNSW with M=16",
    "Structured logging goes through tracing with JSON output in prod",
    "ColumnStore filters are pushed down before the NEAR scan",
    "We squash-merge every pull request into develop",
    "The WASM build drops the tokio dependency",
]


def build_project_memory() -> None:
    """An earlier session recorded the decision, its reason, and routine config."""
    mem = MemoryService(STORE)
    # The reason is a real-world fact; the decision links OUT to it.
    reason = mem.remember(
        "Our biggest customer's field crews work from remote mining sites over satellite links"
    )
    mem.remember(
        "We set the default HTTP request timeout to 7 seconds",
        links=[(reason, "because")],
    )
    for fact in PROJECT_FACTS:
        mem.remember(fact)


def a_new_session_asks_why() -> None:
    """Weeks later a fresh agent reopens the project memory before editing the value."""
    mem = MemoryService(STORE)
    question = "why is the request timeout set to 7 seconds?"

    print(f"  recall({question!r})   — vector similarity, top 5 of 14")
    for hit in mem.recall(question, k=5):
        print(f"     {hit['score']:.2f}  {hit['content']}")
    seen = {hit["content"] for hit in mem.recall(question, k=5)}
    assert not any("mining" in c for c in seen), "recall is blind to the reason"
    print("     └─ the reason is nowhere: it shares no words with the code.\n")

    print(f"  why({question!r})      — vector seed + graph of typed links")
    for node in mem.why(question, max_hops=2)["nodes"]:
        print(f"     hop {node['hop']}  {node['content']}")
    reached = {node["content"] for node in mem.why(question, max_hops=2)["nodes"]}
    assert any("mining" in c for c in reached), "why() must reach the reason"
    print("     └─ why() reached the real reason. Don't round 7 down — you'd")
    print("        cut off the customer on the satellite link.")


if __name__ == "__main__":
    print("── An earlier session recorded the decision and its reason ──\n")
    build_project_memory()
    print("── A new session, about to edit the value, asks why ──\n")
    a_new_session_asks_why()
