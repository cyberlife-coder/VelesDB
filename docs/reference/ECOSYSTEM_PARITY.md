# VelesQL Ecosystem Parity Matrix

Last updated: 2026-04-08 (v1.12.0)

This matrix tracks runtime contract and feature parity across the VelesDB ecosystem.

## Contract Baseline

- Canonical REST contract: `docs/reference/VELESQL_CONTRACT.md`
- Canonical conformance fixture: `conformance/velesql_contract_cases.json`
- Contract version: `3.0.0`

## Endpoint and Payload Parity

| Surface | `/query` | `/aggregate` | `/collections/{name}/match` | Error model (`code/message/hint/details`) | Contract meta |
|---------|----------|--------------|------------------------------|-------------------------------------------|---------------|
| `velesdb-server` | yes | yes | yes | yes | yes (`meta.velesql_contract_version`) |
| TypeScript SDK (REST backend) | yes | yes (auto-routed for aggregate queries) | indirect | yes (nested error parsing) | yes |
| WASM SDK | no (`/query` unsupported by design) | no | no | n/a | n/a |
| CLI (`velesdb-cli`) | yes via server/core path | yes via server/core path | indirect | partial passthrough | partial assertion |
| Python bindings (`velesdb-python`) | core path (non-REST) | core path (non-REST) | core path (non-REST) | n/a REST | n/a REST |
| LangChain integration | via Python binding | via Python binding | via Python binding | n/a REST | n/a REST |
| LlamaIndex integration | via Python binding | via Python binding | via Python binding | n/a REST | n/a REST |

## Feature Parity Matrix (85 features, 10 components)

Legend: ✅ full support | ⚠️ partial / limited | ❌ not supported | N/A not applicable

| Feature Group | Core | Server | Python | WASM | Mobile | CLI | TS SDK | Tauri | LangChain | LlamaIndex |
|---------------|------|--------|--------|------|--------|-----|--------|-------|-----------|------------|
| **Vector CRUD** (insert, upsert, delete, get) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Batch Operations** (batch_insert, batch_upsert) | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Vector Search** (k-NN, filtered, batch) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Multi-Query Fusion** (RRF) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Multi-Query Fusion** (RSF / Weighted) | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ |
| **Hybrid Search** (dense+sparse, dense+text) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Text Search BM25** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Sparse Vector Search** (sparse index) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Sparse Vector Search** (named indexes) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ |
| **Graph Operations** (nodes, edges, traversal) | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Cross-Collection MATCH** (`@collection`) | ✅ | ✅ | ⚠️ | ❌ | ❌ | ✅ | ⚠️ | ❌ | ❌ | ❌ |
| **VelesQL** (parser + executor) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ |
| **Collection Types** (Vector) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Collection Types** (Graph) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Collection Types** (Metadata) | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ |
| **Property Indexes** (secondary, trigram) | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Quantization** (SQ8 / Binary / PQ) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Quantization** (RaBitQ) | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ |
| **Agent Memory** (semantic, episodic, procedural) | ✅ | ⚠️ | ✅ | ✅ | ✅ | N/A | ✅ | ✅ | ⚠️ | ⚠️ |
| **Persistence** (WAL / mmap) | ✅ | ✅ | ✅ | ❌ | ✅ | N/A | N/A | N/A | N/A | N/A |
| **GPU Acceleration** (wgpu) | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

### Notes

- **Cross-Collection MATCH**: Core and Server support `@collection` annotation on MATCH node patterns. Python bindings support via `_collection` param. CLI supports via `\use`. WASM, Mobile, Tauri, and integrations do not yet expose this feature.
- **Batch Operations**: WASM and Mobile use streaming chunked inserts instead of single-call bulk to stay within memory constraints.
- **Multi-Query Fusion (RSF/Weighted)**: WASM supports RRF only; RSF/Weighted fusion is not yet exposed in LangChain/LlamaIndex integrations.
- **Graph Operations (WASM)**: Basic node/edge CRUD is supported; multi-hop traversal and MATCH queries are limited.
- **VelesQL (LangChain/LlamaIndex)**: Pass-through to Python bindings works for simple queries; full parser integration is not surfaced in the integration API.
- **Collection Types (Metadata)**: WASM and integration SDKs expose metadata collections with reduced column-type support.
- **Property Indexes (WASM)**: Disabled by design — no persistence layer means indexes cannot survive page reloads.
- **Quantization (RaBitQ)**: Experimental across all surfaces; API is unstable.
- **Agent Memory (Server)**: Exposed via REST endpoints but not all memory pattern types are fully mapped.
- **Persistence (WASM)**: Disabled by design — `persistence` feature flag is excluded for `wasm32-unknown-unknown` targets.
- **GPU**: Requires `gpu` feature flag; only available in crates that link `wgpu` (core, server, Python bindings).

## Feature Execution Parity (Core Runtime)

| Feature | Parser | Executor | Status |
|---------|--------|----------|--------|
| `SELECT ... FROM ... WHERE ...` | yes | yes | stable |
| `MATCH (...) RETURN ...` | yes | yes | stable |
| `MATCH` via `/query` with `collection` | yes | yes | stable |
| `JOIN ... ON` | yes | yes | stable |
| `JOIN ... USING (...)` | yes | yes (single-column) | stable |
| `LEFT/RIGHT/FULL JOIN` | yes | yes | stable |
| `GROUP BY`, `HAVING` | yes | yes | stable |
| `UNION/INTERSECT/EXCEPT` | yes | yes | stable |

## Conformance Test Coverage

| Surface | Fixture | Test |
|---------|---------|------|
| Server REST contract | `conformance/velesql_contract_cases.json` | `crates/velesdb-server/tests/velesql_conformance_tests.rs` |
| TypeScript SDK contract mapping | `conformance/velesql_contract_cases.json` | `sdks/typescript/tests/velesql-contract-fixtures.test.ts` |
| Core parser | `conformance/velesql_parser_cases.json` | `crates/velesdb-core/tests/velesql_parser_conformance.rs` |
| CLI parser | `conformance/velesql_parser_cases.json` | `crates/velesdb-cli/tests/velesql_parser_conformance.rs` |
| WASM parser | `conformance/velesql_parser_cases.json` | `crates/velesdb-wasm/tests/velesql_parser_conformance.rs` |

## Enum Propagation Matrix

Tracks whether core enums are fully propagated to each ecosystem component.

Legend: ✅ full (all variants) | N/A not applicable (brute-force only, no HNSW)

### DistanceMetric — 9/9 (100%)

All 5 variants (`Cosine`, `Euclidean`, `DotProduct`, `Hamming`, `Jaccard`) are supported in all 9 components.

| Component | Status |
|-----------|--------|
| Core | ✅ (source of truth) |
| Server | ✅ |
| Python | ✅ |
| WASM | ✅ |
| Mobile | ✅ |
| CLI | ✅ |
| TS SDK | ✅ |
| Tauri | ✅ |
| LangChain | ✅ |
| LlamaIndex | ✅ |

### StorageMode — 9/9 (100%)

All 5 variants (`Full`, `SQ8`, `Binary`, `ProductQuantization`, `RaBitQ`) are supported in all 9 components.

| Component | Status |
|-----------|--------|
| Core | ✅ (source of truth) |
| Server | ✅ |
| Python | ✅ |
| WASM | ✅ |
| Mobile | ✅ |
| CLI | ✅ |
| TS SDK | ✅ |
| Tauri | ✅ |
| LangChain | ✅ |
| LlamaIndex | ✅ |

### FusionStrategy — 9/9 (100%)

All 4 strategies (`RRF`, `Weighted`, `Maximum`, `RSF`) plus `Average` are supported in all 9 components.

| Component | Status |
|-----------|--------|
| Core | ✅ (source of truth) |
| Server | ✅ |
| Python | ✅ |
| WASM | ✅ |
| Mobile | ✅ |
| CLI | ✅ |
| TS SDK | ✅ |
| Tauri | ✅ |
| LangChain | ✅ |
| LlamaIndex | ✅ |

### SearchQuality — 6/9

4 HNSW presets (`Fast`, `Balanced`, `Accurate`, `Perfect`) plus `Custom(usize)` and `Adaptive`. WASM, Mobile, and Tauri use brute-force search (no HNSW), so `SearchQuality` is not applicable.

| Component | Status | Notes |
|-----------|--------|-------|
| Core | ✅ (source of truth) | |
| Server | ✅ | |
| Python | ✅ | |
| WASM | N/A | Brute-force only, no HNSW index |
| Mobile | N/A | Brute-force only, no HNSW index |
| CLI | ✅ | |
| TS SDK | ✅ | |
| Tauri | N/A | Brute-force only, no HNSW index |
| LangChain | ✅ | |
| LlamaIndex | ✅ | |

### CollectionType — 8/9

3 types (`Vector`, `MetadataOnly`, `Graph`). Mobile does not expose graph collection creation.

| Component | Status | Notes |
|-----------|--------|-------|
| Core | ✅ (source of truth) | |
| Server | ✅ | |
| Python | ✅ | |
| WASM | ✅ | |
| Mobile | ⚠️ 2/3 | `Vector` and `MetadataOnly` only -- graph collection creation not exposed |
| CLI | ✅ | |
| TS SDK | ✅ | |
| Tauri | ✅ | |
| LangChain | ✅ | |
| LlamaIndex | ✅ | |

### Propagation Summary

| Enum | Coverage | Status |
|------|----------|--------|
| `DistanceMetric` | 9/9 | 100% |
| `StorageMode` | 9/9 | 100% |
| `FusionStrategy` | 9/9 | 100% |
| `SearchQuality` | 6/9 | N/A for WASM/Mobile/Tauri (brute-force) |
| `CollectionType` | 8/9 | Mobile missing graph creation |

---

## Remaining Gaps and Action Items

1. Add explicit CLI end-to-end assertions for REST error shape (`code/hint/details`) beyond parser conformance.
2. Extend WASM conformance from parser-only to executable feature checks where applicable.
3. Keep docs, fixtures, and examples synchronized on every contract version change.
4. Promote RaBitQ from experimental to stable once the API is finalized.
5. Surface RSF/Weighted fusion in LangChain and LlamaIndex integrations.
6. Expose named sparse indexes in LangChain and LlamaIndex integrations.
7. Propagate `@collection` cross-collection MATCH to WASM, Mobile, Tauri, LangChain, and LlamaIndex.
8. Add cross-collection vector search (`similarity()` on `@collection`-annotated nodes).
9. Expose graph collection creation in `velesdb-mobile` (`create_graph_collection`).
