# AGENTS.md — integrations/common (velesdb-common)

Shared utilities for the VelesDB Python integrations. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies here.

- **This package is not a public API** (see its README): helpers here are for the sibling integration packages only — do not document them as user-facing, do not add features "for users" here.
- **Security helpers live here, not reimplemented per integration** (root AGENTS.md rule): path validation at the sink, no `hash()` for IDs, no ReDoS-prone regexes.
- **Tests:** `python3 -m unittest discover -s integrations/common/tests` (picked up by the `python-integrations` CI job).
- Docs: [README.md](README.md).
