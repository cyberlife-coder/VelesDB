# AGENTS.md — integrations/langchain (langchain-velesdb)

LangChain `VectorStore` for VelesDB. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies here.

- **VectorStore API parity with the other integrations:** same constructor options, same `similarity_search*` behavior; the `why()` recall trail is NOT this package's job — link to `langgraph-velesdb` and `crates/velesdb-memory` instead of reimplementing.
- **No permissive validators** (root AGENTS.md): filter dicts are translated to core constraints, unknown keys raise.
- **Tests:** `python3 -m unittest discover -s integrations/langchain/tests` (CI job `python-integrations`).
- Docs: [README.md](README.md).
