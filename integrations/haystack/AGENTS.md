# AGENTS.md — integrations/haystack (haystack-velesdb)

Haystack 2.x `DocumentStore` backed by VelesDB. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies here.

- **Implement the full `DocumentStore` protocol**, not a happy-path subset: `write_documents`, `count_documents`, `delete_documents`, `filter_documents`, and the query paths Haystack calls. A missing method fails at runtime in someone's pipeline, not in CI.
- **Validators are never more permissive than the core** (root AGENTS.md security rule): metadata filters map to VelesQL/ColumnStore constraints, not to ad-hoc Python filtering.
- **Tests:** `python3 -m unittest discover -s integrations/haystack/tests` (CI job `python-integrations` installs the wheel + this package).
- Docs: [README.md](README.md).
