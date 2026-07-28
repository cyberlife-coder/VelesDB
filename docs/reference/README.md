# Technical Reference

In-depth technical documentation. See also the [docs index](../README.md) for
the full documentation map.

| Reference | Description |
|-----------|-------------|
| [Architecture](./ARCHITECTURE.md) | System design and internals |
| [Architecture Diagrams](./ARCHITECTURE_DIAGRAMS.md) | Workspace dependency graph and other architecture diagrams |
| [VelesQL Cheat Sheet](./VELESQL_CHEATSHEET.md) | One-page quick reference: search, filter, graph MATCH, fusion, sparse, EXPLAIN |
| [VelesQL Contract](./VELESQL_CONTRACT.md) | Canonical REST contract (`/query`, `/match`, error model) |
| [VelesQL Conformance](./VELESQL_CONFORMANCE_MATRIX.md) | Cross-ecosystem conformance matrix |
| [Ecosystem Sync Report](./ECOSYSTEM_PARITY.md) | Feature parity matrix across all components |
| [MCP Tool Reference](./MCP_TOOLS.md) | velesdb-memory: every MCP tool, one section each — parameters, returns, limits, error model |
| [Performance SLO](./PERFORMANCE_SLO.md) | CI-enforced performance objectives and budget gates |
| [REST API](./api-reference.md) | HTTP API endpoints |
| [SIMD Performance](./SIMD_PERFORMANCE.md) | SIMD optimizations and benchmarks |
| [Native HNSW](./NATIVE_HNSW.md) | VelesDB's custom native HNSW implementation |
| [Error Codes](./ERROR_CODES.md) | Reference of VelesDB error codes |
| [Known Limitations](./KNOWN_LIMITATIONS.md) | Internal technical limitations of VelesDB Core |
| [Operations Runbook](./OPERATIONS_RUNBOOK.md) | Hybrid query engine operations runbook |

## Machine-readable contracts

| File | Description |
|------|-------------|
| [`promise-contract.json`](./promise-contract.json) | Verbatim-pinned claims (numbers/phrasings) checked against `README.md` and other docs by `scripts/check-promise-contract.py`. Not a doc to read — a gate contract; see the warning comment at the top of the root [`README.md`](../../README.md). |
