# AGENTS.md — integrations/langgraph (langgraph-velesdb)

The `why()` knowledge-graph wedge for LangGraph agents. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies here.

- **This is the package that owns the `why()` / typed-link surface** — keep it thin: it delegates to `velesdb-memory`'s MCP tools, it does not reimplement graph traversal.
- **Checkpointer/state semantics belong to LangGraph, not here:** don't cache memory state in the integration; the store is the source of truth.
- **Tests:** `python3 -m unittest discover -s integrations/langgraph/tests` (CI job `python-integrations`).
- Docs: [README.md](README.md).
