# AGENTS.md — integrations/llamaindex (llama-index-vector-stores-velesdb)

LlamaIndex vector store for VelesDB. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies here.

- **Implement `VectorStore` per LlamaIndex's current protocol** (query/insert/delete with metadata filters) — protocol drift is the main failure mode; pin to the LlamaIndex version in `pyproject.toml` and test against it.
- **No permissive validators** (root AGENTS.md): metadata filters map to core constraints.
- **Tests:** `python3 -m unittest discover -s integrations/llamaindex/tests` (CI job `python-integrations`).
- Docs: [README.md](README.md).
