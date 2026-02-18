# VelesQL Ecosystem Parity Matrix

Last updated: 2026-02-18

This matrix tracks implementation parity of VelesQL contracts and features across the VelesDB ecosystem.

## Contract Baseline

- Canonical REST contract: `docs/reference/VELESQL_CONTRACT.md`
- Canonical conformance cases: `docs/reference/VELESQL_CONFORMANCE_CASES.md`
- Contract version: `2.1.0`

## Endpoint and Payload Parity

| Surface | `/query` | `/collections/{name}/match` | VelesQL Error Model | Contract Version Metadata |
|---------|----------|------------------------------|---------------------|---------------------------|
| `velesdb-server` | ✅ | ✅ | ✅ (`code`, `message/error`, `hint`, `details`) | ✅ (`meta.velesql_contract_version`) |
| TypeScript SDK (REST backend) | ✅ | N/A direct endpoint wrapper | ✅ nested error parsing | ✅ consumes response without break |
| WASM SDK | ❌ (`/query` unsupported by design) | ❌ | N/A | N/A |
| CLI (`velesdb-cli`) | ✅ via server/core query path | Indirect | ⚠️ depends on server payload passthrough | ⚠️ no explicit contract assertion yet |
| Python bindings (`velesdb-python`) | Core path (non-REST) | Core path (non-REST) | N/A REST | N/A REST |
| LangChain integration | Via Python binding | Via Python binding | N/A REST | N/A REST |
| LlamaIndex integration | Via Python binding | Via Python binding | N/A REST | N/A REST |

## Feature Execution Parity (Core Runtime)

| Feature | Parser | Executor | Status |
|---------|--------|----------|--------|
| `SELECT ... FROM ... WHERE ...` | ✅ | ✅ | Stable |
| `MATCH (...) RETURN ...` | ✅ | ✅ | Stable |
| `MATCH` via `/query` with `collection` | ✅ | ✅ | Stable |
| `JOIN ... ON` (inner) | ✅ | ✅ | Stable |
| `JOIN ... USING (...)` | ✅ | ❌ | Parser-only |
| `LEFT/RIGHT/FULL JOIN` | ⚠️ partial/spec | ❌ | Not runtime-ready |
| `GROUP BY`, `HAVING` | ✅ | ✅ | Stable |
| `UNION/INTERSECT/EXCEPT` | ✅ | ✅ | Stable |

## Current Gaps and Action Items

1. Add explicit CLI contract assertions for structured VelesQL errors (`code/hint/details`).
2. Add cross-SDK conformance tests asserting `meta.velesql_contract_version`.
3. Implement runtime support for `JOIN ... USING (...)` before claiming full JOIN parity.
4. Keep docs/README/API examples synchronized whenever contract version changes.
