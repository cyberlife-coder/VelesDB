# AGENTS.md — integrations

Framework surfaces (LangChain, LlamaIndex, Haystack, LangGraph, agent-hooks)
over `velesdb_common`. Root [`AGENTS.md`](../AGENTS.md) is the contract.

- **`velesdb_common` is the shared contract layer.** Validation, fusion
  strategy plumbing, and security helpers live there once — a per-integration
  re-implementation is a parity bug waiting to be found (the
  `weighted_rrf` `weights` filter was exactly this).
- **Client-side validators mirror core.** `validate_collection_name` must
  admit exactly what `velesdb-core::validate_collection_name` admits — cap,
  charset, leading-hyphen and reserved-name rules included. A looser client
  validator fails at first write, not at validation time.
- **Tests:** `PYTHONPATH=integrations/common/src python3 -m pytest
  integrations/common/tests` (the package is not pip-installed in a
  worktree); agent-hooks shell tests run via
  `bash integrations/agent-hooks/test/hooks.test.sh`.
