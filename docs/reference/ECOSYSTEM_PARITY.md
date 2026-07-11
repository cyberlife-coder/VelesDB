# VelesQL Ecosystem Parity Matrix

Last updated: 2026-07-12 (v3.9.1; velesdb-memory 0.6.0)

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
| Haystack integration | via Python binding | via Python binding | via Python binding | n/a REST | n/a REST |

## Feature Parity Matrix (86 features, 11 components)

Legend: ✅ full support | ⚠️ partial / limited | ❌ not supported | N/A not applicable

| Feature Group | Core | Server | Python | WASM | Mobile | CLI | TS SDK | Tauri | LangChain | LlamaIndex | Haystack |
|---------------|------|--------|--------|------|--------|-----|--------|-------|-----------|------------|----------|
| **Vector CRUD** (insert, upsert, delete, get) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Batch Operations** (batch_insert, batch_upsert) | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Streaming Ingestion** (enableStreaming / stream_insert) | ✅ | ✅ | ✅ | ❌ | ✅ | ⚠️ | ✅ | ✅ | ⚠️ | ⚠️ | ❌ |
| **Vector Search** (k-NN, filtered, batch) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Multi-Query Fusion** (RRF) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ |
| **Multi-Query Fusion** (RSF / Weighted) | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ |
| **Hybrid Search** (dense+sparse, dense+text) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ |
| **Text Search BM25** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ |
| **Sparse Vector Search** (sparse index) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| **Sparse Vector Search** (named indexes) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ❌ |
| **Graph Operations** (nodes, edges, traversal) | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | N/A |
| **Cross-Collection MATCH** (`@collection`) | ✅ | ✅ | ⚠️ | ❌ | ❌ | ✅ | ⚠️ | ❌ | ❌ | ❌ | ❌ |
| **VelesQL** (parser + executor) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ⚠️ |
| **Collection Types** (Vector) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Collection Types** (Graph) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | N/A |
| **Collection Types** (Metadata) | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ⚠️ |
| **Property Indexes** (secondary, trigram) | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ |
| **Quantization** (SQ8 / Binary / PQ) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Quantization** (RaBitQ) | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ |
| **Agent Memory** (semantic, episodic, procedural) | ✅ | ⚠️ | ✅ | ⚠️ | ⚠️ | N/A | ✅ | ✅ | ⚠️ | ⚠️ | N/A |
| **Persistence** (WAL / mmap) | ✅ | ✅ | ✅ | ❌ | ✅ | N/A | N/A | N/A | N/A | N/A | N/A |
| **GPU Acceleration** (wgpu) | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

### Notes

- **Cross-Collection MATCH**: Core and Server support `@collection` annotation on MATCH node patterns. Python bindings support via `_collection` param. CLI supports via `\use`. WASM, Mobile, Tauri, and integrations do not yet expose this feature.
- **Batch Operations**: WASM and Mobile use streaming chunked inserts instead of single-call bulk to stay within memory constraints. WASM additionally exposes a single-call raw-bulk path via `VectorStore.insertBatchRaw` (see the Raw-Bulk Insert note).
- **Streaming Ingestion** (2026-06-14): Core, Server (`POST /collections/{name}/stream/enable` + `/stream/insert`), Python, Tauri, Mobile (`enableStreaming()` / `streamInsert()` on its own tokio streaming runtime), and the TS SDK (`enableStreaming()` / `streamInsert()`, REST backend) support the bounded ingestion channel. The CLI reaches it ⚠️ via the embedded core path with no dedicated REPL command. WASM throws `NOT_SUPPORTED` (no persistence layer). LangChain and LlamaIndex expose ⚠️ streaming via `stream_insert()`/`add_texts_streaming()`/`add_streaming()`, which forward to `collection.stream_insert` (caller is responsible for `enable_streaming`; covered by mock-collection unit tests). Only the Haystack integration does not yet expose it.
- **Raw-Bulk Insert** (2026-06-14): the zero-copy raw-bulk path is now exposed by Core (`upsert_bulk_from_raw`), Server (`POST /collections/{name}/points/raw`, VRB1 binary), the TS SDK (`upsertBatchRaw`), WASM (`VectorStore.insertBatchRaw(ids, vectors, dim)`, writing into its in-memory buffer), and the CLI (`velesdb data import <file.bin>`, VRB1 binary). Mobile remains a follow-up. All surfaces share the one `velesdb_core::wire::vrb1` codec.
- **Multi-Query Fusion (RSF/Weighted)** (2026-06-14): WASM's multi-query `fuse_results` now delegates **4 of its 5 strategies** (`average`, `maximum`, `weighted`, `rrf`) to the canonical `velesdb_core::FusionStrategy::fuse`, so the browser engine reproduces core's ranking 1:1 for those (`crates/velesdb-wasm/src/fusion.rs`; equivalence pinned by `test_fuse_results_matches_core_ordering`). The fifth strategy, `relative_score` / `rsf`, is **intentionally kept WASM-local**: core's `RelativeScore` is a two-branch (dense + sparse) weighted sum that zero-fills documents missing from a branch and discards branches beyond index 1, whereas WASM's is an N-branch equal-weight average that skips missing branches. The two semantics yield different rankings, so converging WASM onto core would silently change WASM search results — that convergence is a product decision deferred to a follow-up, and `relative_score` behaviour is unchanged. (This is the multi-query fusion entry point; the VelesQL `USING FUSION (...)` clause executor in `velesql_fusion.rs` already builds every strategy — including RSF — directly from `velesdb_core::fusion::FusionStrategy`.) LangChain and LlamaIndex expose RSF/Weighted through `multi_query_search(fusion=...)`, which delegates to the shared `velesdb_common.fusion.build_fusion_strategy` (builds `weighted()` and `relative_score()`). Haystack reaches RSF/Weighted/RRF/etc. fusion via its own `VelesDBDocumentStore.embedding_retrieval(fusion=..., fusion_params=...)`, which delegates to `velesdb_common.fusion.build_fusion_strategy` and `Collection.multi_query_search`.
- **Sparse Vector Search (named indexes) — LangChain/LlamaIndex**: ⚠️ query-side only. Both integrations forward a `sparse_index_name` argument to the underlying `collection.search`/`hybrid_search`, so an existing named sparse index can be *queried*. Named sparse indexes are also created on the upsert/write path: passing a named mapping such as `{"bge_m3": {0: 1.5}}` to `add_texts`/`add`/`add_bulk` (LC/LI) or `write_documents` (Haystack) creates the named index, validated by `velesdb_common.security.validate_named_sparse_vector`.
- **Graph Operations (WASM)**: Basic node/edge CRUD is supported; multi-hop traversal and MATCH queries are limited.
- **VelesQL (LangChain/LlamaIndex/Haystack)**: Pass-through to Python bindings works for simple queries; full parser integration is not surfaced in the integration API.
- **Haystack DocumentStore protocol limits**: The Haystack 2.x `DocumentStore` ABC exposes `write_documents`, `filter_documents`, `embedding_retrieval`, `count_documents`, and `delete_documents`. BM25 / hybrid retrieval requires a separate `Retriever` component (planned follow-up). Graph collections, agent memory, and sparse-named indexes are intentionally `N/A` because they have no idiomatic mapping in Haystack's protocol and are reachable through the raw `velesdb` Python wrapper if needed.
- **Collection Types (Metadata)**: WASM and integration SDKs expose metadata collections with reduced column-type support.
- **Property Indexes (WASM)**: Disabled by design — no persistence layer means indexes cannot survive page reloads.
- **Quantization (RaBitQ)**: Experimental across all surfaces; API is unstable.
- **Agent Memory (Server)**: ⚠️ — durable point TTL **is** exposed over REST
  (`PATCH /collections/{name}/points/{id}/ttl`, persisted as
  `_veles_expires_at` and enforced on every read surface — search/get/scroll/
  query/MATCH), and relation edges are managed via
  `POST /collections/{name}/relations`, `DELETE .../relations/{edge_id}`, and
  `GET .../points/{id}/relations`. Still embedded-only: temporal/confidence-only
  queries, reinforcement, and snapshots.
  Per-binding parity for the relation + durable-TTL surface:

  | Operation | REST | TS SDK (REST backend) | TS SDK (WASM backend) | Python |
  |---|---|---|---|---|
  | `relate()` (create edge) | ✅ `POST .../relations` | ✅ `client.relate()` | ❌ (`wasmRelate` throws `NOT_SUPPORTED` — REST backend only) | ❌ (use `GraphCollection.add_edge` or the core API) |
  | `unrelate()` (delete edge) | ✅ `DELETE .../relations/{edge_id}` | ✅ `client.unrelate()` | ❌ (throws `NOT_SUPPORTED`) | ❌ |
  | `getRelations()` (list outgoing) | ✅ `GET .../points/{id}/relations` | ✅ `client.getRelations()` | ❌ (throws `NOT_SUPPORTED`) | ❌ |
  | Durable TTL set/refresh | ✅ `PATCH .../points/{id}/ttl` | ✅ `client.setTtlDurable()` | ❌ (throws `NOT_SUPPORTED`) | ✅ `set_semantic/episodic/procedural_ttl_durable`, `store_with_ttl`, `record_with_ttl`, `learn_with_ttl` |
  | Temporal recall facades | n/a (use `/query`) | ✅ `recallRecent` / `recallOlderThan` | ❌ (throws `NOT_SUPPORTED`) | ✅ `episodic.recent` / `episodic.older_than` |
- **Agent Memory (WASM / Mobile)**: WASM now ships the high-level `MemoryService` wedge (remember — incl. per-fact `ttlSeconds` — /recall/recallWhere/recallFused/relate/forget/why; `remember_extracted` excluded, it needs a generative model; in-memory only, no persistence under WASM, #1310) alongside the primitive `SemanticMemory`; mobile remains ⚠️ semantic-only (`VelesSemanticMemory`). Episodic/procedural memory, the standalone TTL setters (`setTtlDurable`-style), and snapshots are not exposed on these bindings.
- **Auto-extraction (text → graph)**: lives in the high-level `velesdb-memory` **MCP server**, not in this core-feature matrix. `MemoryService::remember_extracted` (and the `remember_extracted` MCP tool) run an `Extractor` over raw text and auto-wire the fact↔topic graph; the reusable core primitive it builds on is `SemanticMemory::query_excluding` (negative-filter vector search, used to keep internal entity hubs out of recall/why). The MCP `recall`/`why` tools inherit hub-exclusion transparently. The high-level `MemoryService` wedge (remember/recall/recall_where/relate/forget/why/remember_extracted) is now exposed beyond the MCP server in **Python** (`velesdb-python`, #1242), **Node.js** (`velesdb-node` / npm `@wiscale/velesdb-memory-node`, #1245), and the **TS/WASM SDK** (`MemoryService` over the in-browser backend, #1310 — `remember_extracted` excluded there, as it needs a generative model).
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
| Core executor (rows/counts/ordering) | `conformance/velesql_executor_cases.json` | `crates/velesdb-core/tests/velesql_executor_conformance.rs` |
| CLI executor (rows/counts/ordering) | `conformance/velesql_executor_cases.json` | `crates/velesdb-cli/tests/velesql_executor_conformance.rs` |
| WASM executor (rows/counts/ordering) | `conformance/velesql_executor_cases.json` | `crates/velesdb-wasm/src/velesql_executor_conformance_tests.rs` |
| Core parser | `conformance/velesql_parser_cases.json` | `crates/velesdb-core/tests/velesql_parser_conformance.rs` |
| CLI parser | `conformance/velesql_parser_cases.json` | `crates/velesdb-cli/tests/velesql_parser_conformance.rs` |
| WASM parser | `conformance/velesql_parser_cases.json` | `crates/velesdb-wasm/tests/velesql_parser_conformance.rs` |

The executor fixture (added 2026-06-14, extended 2026-06-20) asserts the exact
result set (ids, count, ordering) each executor produces for a fixed dataset. As
of 2026-06-20 the **WASM and CLI executors are fixture-checked against the same
goldens too** — the CLI drives the real binary end-to-end and WASM runs its own
SELECT/ORDER BY pipeline — so a result-shape divergence on those surfaces fails
CI rather than going unnoticed. Coverage includes scalar WHERE filters, single-
and multi-column ORDER BY, the ascending-id tie-break, and bounded top-k
(`ORDER BY ... LIMIT k`); see
[KNOWN_LIMITATIONS #13](./KNOWN_LIMITATIONS.md#13-velesql-executor-conformance-core-wasm-cli)
(resolved).

## Enum Propagation Matrix

Tracks whether core enums are fully propagated to each ecosystem component.

Legend: ✅ full (all variants) | N/A not applicable (brute-force only, no HNSW)

### DistanceMetric — 10/10 (100%)

All 5 variants (`Cosine`, `Euclidean`, `DotProduct`, `Hamming`, `Jaccard`) are supported in all 10 components (Haystack inherits via the Python binding pass-through).

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
| Haystack | ✅ |

### StorageMode — 10/10 (100%)

All 5 variants (`Full`, `SQ8`, `Binary`, `ProductQuantization`, `RaBitQ`) are supported in all 10 components (Haystack inherits via the Python binding pass-through).

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
| Haystack | ✅ |

### FusionStrategy — 10/10 (100%)

All 4 strategies (`RRF`, `Weighted`, `Maximum`, `RSF`) plus `Average` are supported in all 10 components (Haystack reaches RSF/Weighted/RRF/etc. fusion via its own `VelesDBDocumentStore.embedding_retrieval(fusion=..., fusion_params=...)`, which delegates to `velesdb_common.fusion.build_fusion_strategy` and `Collection.multi_query_search`).

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
| Haystack | ✅ |

### SearchQuality — 7/10

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
| Haystack | ✅ | |

### CollectionType — 9/10

3 types (`Vector`, `MetadataOnly`, `Graph`). All native crates expose graph
collection creation; only Haystack is limited by its DocumentStore protocol.

| Component | Status | Notes |
|-----------|--------|-------|
| Core | ✅ (source of truth) | |
| Server | ✅ | |
| Python | ✅ | |
| WASM | ✅ | |
| Mobile | ✅ | `create_graph_collection` / `create_graph_collection_with_embeddings` exposed via `#[uniffi::export]` |
| CLI | ✅ | |
| TS SDK | ✅ | |
| Tauri | ✅ | |
| LangChain | ✅ | |
| LlamaIndex | ✅ | |
| Haystack | ⚠️ 1/3 | `Vector` only — `Graph` and `MetadataOnly` have no idiomatic mapping in the Haystack DocumentStore protocol |

### Propagation Summary

| Enum | Coverage | Status |
|------|----------|--------|
| `DistanceMetric` | 10/10 | 100% |
| `StorageMode` | 10/10 | 100% |
| `FusionStrategy` | 10/10 | 100% |
| `SearchQuality` | 7/10 | N/A for WASM/Mobile/Tauri (brute-force) |
| `CollectionType` | 9/10 | Haystack `Vector` only by protocol; all native crates full |

---

## Recently Landed (2026-06-14)

- **WASM fusion now delegates 4/5 strategies to core.** `average`/`maximum`/`weighted`/`rrf` map onto `velesdb_core::FusionStrategy::fuse` (ranking identical to core, pinned by an equivalence test); `relative_score`/`rsf` stays WASM-local by design because its N-branch equal-weight semantics differ from core's two-branch dense+sparse weighted sum. See the RSF/Weighted note above.
- **Executor-level conformance now exists for core.** `conformance/velesql_executor_cases.json` + `crates/velesdb-core/tests/velesql_executor_conformance.rs` assert result rows/counts/ordering (not just that a query parses). Extended to the WASM and CLI executors on 2026-06-20 (action item 2 — now done).
- **Scalar `ORDER BY` + `LIMIT` correctness bug fixed.** A scalar (non-`similarity()`) `ORDER BY <col> ... LIMIT k` previously truncated to `k` in storage order *before* sorting; it now fetches the full matching set so the sort precedes truncation, restoring the [KNOWN_LIMITATIONS #9](./KNOWN_LIMITATIONS.md#9-bounded-query-result-materialization) bounded==unbounded guarantee. The `similarity()`-ordered HNSW fast path was untouched, so recall is unaffected. (Surfaced by the new executor conformance net above.)
- **Point-ID hashing single-sourced for Haystack.** The Haystack `DocumentStore` now imports the canonical `velesdb_common.ids.stable_hash_id` instead of a bit-identical forked copy (behaviour-preserving; removes a re-implemented hash from an MIT package). The intentional remaining divergence — `velesdb-migrate`'s distinct `stable_point_id` — is documented in [KNOWN_LIMITATIONS #12](./KNOWN_LIMITATIONS.md#12-string--u64-point-id-hashing-differs-across-components).

## Remaining Gaps and Action Items

1. ✅ **Done (2026-06-24).** Explicit server-side assertions for the full REST `VelesqlErrorResponse` shape (`code`/`message`/`hint`/`details`) are now enforced in `crates/velesdb-server/tests/velesql_conformance_tests.rs` via the shared fixture. Cases C002 (`VELESQL_MISSING_COLLECTION`), C003 (`VELESQL_COLLECTION_NOT_FOUND`), and C007 (`VELESQL_AGGREGATION_ERROR`) each assert all four fields, pinning the complete error body contract for the `/query` and `/aggregate` semantic-error paths. Parse errors (`QueryErrorResponse`) retain their own parser-specific shape and are tested separately by C004/C013 (status code only, by design — the parser error format is defined at `E0XX` layer). (The CLI has no HTTP layer — it executes against embedded core — so this contract belongs exclusively to `velesdb-server`.)
2. ✅ **Done (2026-06-20).** The executor-level conformance net now covers **core, WASM, and CLI** — all three run `conformance/velesql_executor_cases.json`, including scalar WHERE filters, single- and multi-column ORDER BY, the ascending-id tie-break, and bounded top-k. See [KNOWN_LIMITATIONS #13](./KNOWN_LIMITATIONS.md#13-velesql-executor-conformance-core-wasm-cli) (resolved).
3. Keep docs, fixtures, and examples synchronized on every contract version change.
4. Promote RaBitQ from experimental to stable once the API is finalized.
5. ✅ **Done.** RSF/Weighted fusion is exposed in Haystack via
   `embedding_retrieval(fusion=...)` through `velesdb_common.fusion` (already
   exposed in LangChain and LlamaIndex via the shared `velesdb_common.fusion`
   module).
6. ✅ **Done.** Named-sparse-index *creation* is exposed on the upsert/write path
   of LangChain, LlamaIndex, and Haystack (query-side `sparse_index_name`
   targeting already works).
7. Propagate `@collection` cross-collection MATCH to WASM, Mobile, Tauri, LangChain, LlamaIndex, and Haystack.
8. Add cross-collection vector search (`similarity()` on `@collection`-annotated nodes).
