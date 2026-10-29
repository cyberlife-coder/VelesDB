# VelesQL Ecosystem Parity Matrix

Last updated: 2026-10-29 (v7.1.0 + unreleased fusion/search-mode grammar; velesdb-memory 0.16.0)

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
| **Streaming Ingestion** (enableStreaming / stream_insert) | ✅ | ✅ | ✅ | ❌ | ✅ | ⚠️ | ✅ | ✅ | ⚠️ | ⚠️ | ⚠️ |
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

- **Cross-Collection MATCH**: Core and Server support `@collection` annotation on MATCH node patterns. Python bindings support via `_collection` param. CLI supports via `\use`. Mobile reaches it through `execute_query` (full VelesQL pass-through, `crates/velesdb-mobile/src/lib.rs:342`). WASM, Tauri, and integrations do not yet expose this feature.
- **Batch Operations**: WASM and Mobile use streaming chunked inserts instead of single-call bulk to stay within memory constraints. WASM additionally exposes a single-call raw-bulk path via `VectorStore.insertBatchRaw` (see the Raw-Bulk Insert note).
- **Streaming Ingestion** (2026-07-23): Core, Server (`POST /collections/{name}/stream/enable` + `/stream/insert`), Python, Tauri, Mobile (`enableStreaming()` / `streamInsert()` on its own tokio streaming runtime), and the TS SDK (`enableStreaming()` / `streamInsert()`, REST backend) support the bounded ingestion channel. The CLI reaches it ⚠️ via the embedded core path with no dedicated REPL command. WASM throws `NOT_SUPPORTED` (no persistence layer). LangChain and LlamaIndex expose ⚠️ streaming via `stream_insert()`/`add_texts_streaming()`/`add_streaming()`, which forward to `collection.stream_insert` (caller is responsible for `enable_streaming`; covered by mock-collection unit tests). Haystack now exposes the same ⚠️ shape via `VelesDBDocumentStore.stream_insert()`/`write_documents_streaming()` (issue #1548), forwarding to `collection.stream_insert` with the same caller-managed `enable_streaming` contract.
- **Raw-Bulk Insert** (2026-06-14): the zero-copy raw-bulk path is now exposed by Core (`upsert_bulk_from_raw`), Server (`POST /collections/{name}/points/raw`, VRB1 binary), the TS SDK (`upsertBatchRaw`), WASM (`VectorStore.insertBatchRaw(ids, vectors, dim)`, writing into its in-memory buffer), and the CLI (`velesdb data import <file.bin>`, VRB1 binary). Mobile remains a follow-up. All surfaces share the one `velesdb_core::wire::vrb1` codec.
- **Multi-Query Fusion (RSF/Weighted)** (2026-06-14): WASM's multi-query `fuse_results` now delegates **4 of its 5 strategies** (`average`, `maximum`, `weighted`, `rrf`) to the canonical `velesdb_core::FusionStrategy::fuse`, so the browser engine reproduces core's ranking 1:1 for those (`crates/velesdb-wasm/src/fusion.rs`; equivalence pinned by `test_fuse_results_matches_core_ordering`). The fifth strategy, `relative_score` / `rsf`, is **intentionally kept WASM-local**: core's `RelativeScore` is a two-branch (dense + sparse) weighted sum that zero-fills documents missing from a branch and discards branches beyond index 1, whereas WASM's is an N-branch equal-weight average that skips missing branches. The two semantics yield different rankings, so converging WASM onto core would silently change WASM search results — that convergence is a product decision deferred to a follow-up, and `relative_score` behaviour is unchanged. (This is the multi-query fusion entry point; the VelesQL `USING FUSION (...)` clause executor in `velesql_fusion.rs` already builds every strategy — including RSF — directly from `velesdb_core::fusion::FusionStrategy`.) LangChain and LlamaIndex expose RSF/Weighted through `multi_query_search(fusion=...)`, which delegates to the shared `velesdb_common.fusion.build_fusion_strategy` (builds `weighted()` and `relative_score()`). Haystack reaches RSF/Weighted/RRF/etc. fusion via its own `VelesDBDocumentStore.embedding_retrieval(fusion=..., fusion_params=...)`, which delegates to `velesdb_common.fusion.build_fusion_strategy` and `Collection.multi_query_search`.
- **Sparse Vector Search (named indexes) — LangChain/LlamaIndex**: ⚠️ query-side only. Both integrations forward a `sparse_index_name` argument to the underlying `collection.search`/`hybrid_search`, so an existing named sparse index can be *queried*. Named sparse indexes are also created on the upsert/write path: passing a named mapping such as `{"bge_m3": {0: 1.5}}` to `add_texts`/`add`/`add_bulk` (LC/LI) or `write_documents` (Haystack) creates the named index, validated by `velesdb_common.security.validate_named_sparse_vector`.
- **LangGraph integration** (2026-07-23, updated #1546): `integrations/langgraph` ships ten memory tools via `make_memory_tools` — `remember` (with `links`/`metadata`/`ttl_seconds`), `recall`, `recall_where`, `recall_fused` (incl. `date_field=` dated timelines), `relate`, `forget`, `feedback`, `why`, and `save_working_context`/`load_working_context`. That is not the whole wedge: `entity`, `unrelate`, and the context-compiler tools are not exposed (see the memory-surface gap block below). It is deliberately narrow — no vector-store surface — and is not yet a column in the matrices above. Remaining gap: `list_working_contexts` is not exposed by the LangGraph toolkit, though the `velesdb` Python binding (`MemoryService.list_working_contexts`) does have it since 0.11.x — `load_working_context`'s `other_sessions` now covers the typo-recovery case the listing was mainly wanted for. The package's `velesdb>=3.12.0` floor is deliberately kept there pending the 5.0.0 release that carries the `load_working_context` envelope (CHANGELOG [Unreleased]) — later `velesdb` releases exist on PyPI, but floors move with the 5.0.0 release train; `feedback`, `save_working_context`, `load_working_context`, and the `_veles_date` auto-stamp below landed in source after the 3.12.0 cut and need that release to work (see the integration's README "Compatibility" section).
- **Auto `_veles_date` stamp (introduced in velesdb-memory 0.11.0)**: every `remember` now auto-stamps a reserved `_veles_date` metadata field (`YYYYMMDD`, caller-set value never overwritten), making `recall_fused(date_field=…)` work with zero setup. Surfaced by delegation in Python (`tests/test_memory_service.py` asserts the stamp) and Node (`recallFusedDated`). **WASM is read-side only**: no clock exists on `wasm32-unknown-unknown`, so the WASM write path never stamps `_veles_date` (`MemoryService::remember_with_ttl`'s contract) — `recallFusedDated` exists there, but zero-setup dating does not; a caller must set `_veles_date` explicitly for the timeline to fill. The TS SDK (which wraps WASM) types and documents the `_veles_date` key (`sdks/typescript/src/memory.ts`, `MemoryMetadata._veles_date`) and ships `recallFusedDated`, under the same explicit-stamp caveat. LangGraph now exposes `recall_where`/`recall_fused(date_field=…)` to reach it too (see above), pending a `velesdb` release past 3.12.0.
- **`load_working_context` returns an ENVELOPE on every surface since velesdb-memory 0.12.0**: `{found, working, other_sessions}` — the MCP tool, the Node, Python and WASM bindings, and the TS SDK. **Breaking** for the three bindings and the SDK, which until 0.12.0 returned the bare working context (or `null`/`None`): read `.working` for that value. The bare form could not express the difference between "nothing was ever saved" and "a typo in `session` missed a session that exists" — `other_sessions` names the near-misses, and is filled in on a hit too, since a typo landing on another REAL session is the case a caller can least detect. Shape parity is now enforced, not just method-name parity: `crates/velesdb-memory/tests/binding_parity_bdd.rs` reads each tool's `output_schema` root keys off the LIVE server and requires every binding to relay them or declare the divergence. Three pre-existing divergences it surfaced were declared there as known gaps (six entries, since two of them spanned more than one binding): all three bindings dropped `entity.relations_in` (added server-side by #1681), the Node binding's `CompiledContextJs` dropped `compile_context.warnings`, and both the Node and Python bindings dropped `remember_extracted.skipped_over_cap`. **All six are closed** (#1690, #1691, #1692); `SHAPE_DIVERGENCES` now holds deliberate unwraps only.
- **The TypeScript SDK is inside a guard's perimeter (#1721)**: it had never been in one, and shipped 17 of the 19 methods the WASM binding exposed for its whole life — `entity` and `unrelate` were simply absent. It is checked as a SECOND LINK rather than as a fourth binding: the SDK wraps the WASM bundle and can never publish more than it does, so the invariant is "the SDK relays every tool that reaches WASM", composed with the existing "WASM relays every tool the server advertises". Framing it as a fourth binding would have duplicated WASM's `feedback` exemption into a second list — and a decision recorded twice is a decision that will drift. The reader is a small TypeScript-aware scan in the same file, because the tool list comes from a REAL running MCP server; a check living in the SDK's own test suite would have had to hard-code that list, which is the very defect #1721 reports.
- **Hybrid Search (dense + sparse) — one fusion in core** (2026-10-29): `VectorCollection::hybrid_sparse_search` / `hybrid_sparse_search_filtered`, the entry point the server, CLI, Python, mobile and Tauri call, carried its own copy of the dense + sparse fusion without the #2102 score-direction alignment, so on a Euclidean or Hamming collection a score-level strategy ranked the farthest dense point first. It now delegates to `Collection::hybrid_sparse_search_inner`, the body the VelesQL `NEAR ... AND SPARSE_NEAR ...` path fuses through. On every one of those surfaces a dense vector of the wrong dimension fails with `DimensionMismatch` (VELES-004) and an unknown sparse index with `Sparse index '...' not found` (VELES-009), as the sparse-only search already did; both used to degrade to the other branch. WASM's in-memory `hybrid_search_fuse` is a separate path and unchanged.
- **VelesQL `NEAR` under `OR`** (2026-10-29): core and WASM agree. `vector NEAR $v OR <metadata>` returns the NEAR ranking with NEAR scores and reads the rest of the WHERE with the NEAR leaf as TRUE — what the WASM executor already did, and what core's WHERE evaluator says. Core used to drop the NEAR hits. Executor fixture cases X015–X016 run on core, CLI and WASM.
- **`NEAR_FUSED ... USING FUSION 'weighted'|'rsf'`** (known gap): core's validator rejects it with `V012`; the WASM executor does not run core's `QueryValidator` and executes it as `rrf` (`crates/velesdb-wasm/src/velesql_fused.rs`, `config_to_strategy`).
- **Graph Operations (WASM)**: Basic node/edge CRUD is supported; multi-hop traversal and MATCH queries are limited.
- **VelesQL (LangChain/LlamaIndex/Haystack)**: Pass-through to Python bindings works for simple queries; full parser integration is not surfaced in the integration API.
- **Haystack DocumentStore protocol limits**: The Haystack 2.x `DocumentStore` ABC exposes `write_documents`, `filter_documents`, `embedding_retrieval`, `count_documents`, and `delete_documents`. Dense retrieval in pipelines ships as the dedicated `VelesDBEmbeddingRetriever` component (`integrations/haystack/src/haystack_velesdb/retriever.py`); BM25/hybrid retriever components remain a follow-up. Graph collections, agent memory, and sparse-named indexes are intentionally `N/A` because they have no idiomatic mapping in Haystack's protocol and are reachable through the raw `velesdb` Python wrapper if needed.
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
- **Agent Memory (WASM / Mobile)**: WASM now ships the high-level `MemoryService` wedge (remember — incl. per-fact `ttlSeconds` — /recall/recallWhere/recallFused/relate/unrelate/forget/entity/why/rememberExtracted; in-memory only, no persistence under WASM, #1310) alongside the primitive `SemanticMemory`. `rememberExtracted` on WASM (and on the TS SDK wrapping it) runs the deterministic, network-free `outline` extractor ONLY — a generative backend (`"ollama"`) is refused by name rather than silently substituted, because it would put a network call in the bundle (`crates/velesdb-wasm/src/memory_service.rs`, `sdks/typescript/src/memory.ts` `rememberExtracted`). `feedback` remains deliberately absent on WASM (it sits behind the `persistence` feature; a durable learned confidence is meaningless for a store that disappears on page reload). Mobile remains ⚠️ semantic-only (`VelesSemanticMemory`). Episodic/procedural memory, the standalone TTL setters (`setTtlDurable`-style), and snapshots are not exposed on these bindings.
- **Agent Memory — search quality per call (velesdb-core 7.1.0)**: `SemanticMemory::query_with_quality`, `query_filtered_with_quality` and `query_excluding_with_quality` run one semantic search at an explicit `SearchQuality`; `query`, `query_filtered` and `query_excluding` keep the collection's runtime quality. Usage: [AGENT_MEMORY.md](../guides/AGENT_MEMORY.md#semantic-search-at-an-explicit-quality). Where the choice reaches a caller:

  | Surface | Per-call quality | Per-store quality |
  |---|---|---|
  | Core (`SemanticMemory`) | ✅ the three `*_with_quality` methods | n/a |
  | `velesdb-memory` MCP server | n/a | ✅ `VELESDB_MEMORY_SEARCH_QUALITY` / `[search] quality` ([MCP_SERVER_SETUP.md](../guides/MCP_SERVER_SETUP.md#recall-quality)) |
  | Python `MemoryService` | n/a | ✅ `MemoryService(path, ..., search_quality=...)` |
  | Node `MemoryService` | n/a | ✅ `searchQuality` |
  | Python `AgentMemory.semantic` | ❌ `query` takes no quality | ❌ |
  | Tauri plugin, Mobile `VelesSemanticMemory` | ❌ the semantic query command / `query` takes no quality | ❌ |
  | WASM / TS SDK `MemoryService` | N/A — brute-force search, no HNSW (see `SearchQuality` below) | N/A |

  All per-store settings parse the value with `velesdb_core::api_types::parse_search_mode`, so the accepted forms and the refusal of an unknown one are the same everywhere. `recall_where` / column-predicate recall runs its own search path and is not affected.
- **Auto-extraction (text → graph)**: lives in the high-level `velesdb-memory` **MCP server**, not in this core-feature matrix. `MemoryService::remember_extracted` (and the `remember_extracted` MCP tool) run an `Extractor` over raw text and auto-wire the fact↔topic graph; MCP accepts the backend per call, with the daemon's configured backend as the default and `outline` always available. The reusable core primitive it builds on is `SemanticMemory::query_excluding` (negative-filter vector search, used to keep internal entity hubs out of recall/why). The MCP `recall`/`why` tools inherit hub-exclusion transparently. The high-level `MemoryService` wedge (remember/recall/recall_where/relate/forget/why/remember_extracted) is now exposed beyond the MCP server in **Python** (`velesdb-python`, #1242), **Node.js** (`velesdb-node` / npm `@wiscale/velesdb-memory-node`, #1245), and the **TS/WASM SDK** (`MemoryService` over the in-browser backend, #1310 — `rememberExtracted` included there via the deterministic `outline` extractor only; a generative backend is refused, see the Agent Memory (WASM / Mobile) note).
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

> **Documented divergence — the memory behaviour of `SQ8` and `RaBitQ` is not uniform across surfaces.** The API is: every component accepts and persists all 5 variants, which is what the 10/10 above measures. What differs is what the mode *does* to the resident set.
>
> Since #2112 those two modes keep their f32 vectors in a file-backed arena rather than an anonymous allocation, cutting anonymous RSS 61% at 100 000 × 768-d (see [QUANTIZATION.md](../guides/QUANTIZATION.md#measured-resident-set)). That arena is gated on the `persistence` feature and needs a writable collection directory. A native host that cannot host the arena file (read-only directory, no space) runs those modes with a heap arena and no resident-set saving, by design and with a `warn!`.
>
> **WASM: same mode name, same results, no memory saving.** The browser `VectorStore` does not use core's index: it searches by brute force. Every storage mode keeps the f32 vectors (4 bytes per dimension plus 8 per id, as `Full`) and scores them exactly, so each mode returns the same top-k and scores as a core collection created with it — core re-ranks its quantized modes with exact f32 and runs `Binary` as `Full`. `crates/velesdb-wasm/src/quantization_parity_tests.rs` pins this per mode and metric against a real core collection (CI's "WASM↔core quantization parity" step). This was not true until the change that introduced that test: the browser store kept only lossy codes (a per-vector SQ8 for `SQ8`/`ProductQuantization`/`RaBitQ`, sign bits decoded to 0/1 for `Binary`) and scored the reconstruction, so the same mode name ranked differently from core. Stores saved in that format (`VELS` v2) still load, as their reconstruction.
>
> One core-side split remains, and the browser follows core's exact side of it: once a `ProductQuantization` collection has trained its quantizer (128 inserts), core's default `search()` returns ADC-approximated scores over an oversampled candidate set, while `search_with_quality` returns exact f32 scores (`collection/search/vector.rs`). The browser store returns the exact scores in both entry points.
>
> Nothing to fix in the bindings: they pass the mode through and core decides. It is recorded here because "supported everywhere" is true of the surface and would be misread as true of the benefit.

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

### SearchQuality — 9/10

3 HNSW presets (`Fast`, `Balanced`, `Accurate`); `Perfect`, an exhaustive scan that leaves the graph and is capped by `limits.max_perfect_mode_vectors`; plus `Custom(usize)`, `Adaptive` and `AutoTune`. WASM uses brute-force search (no HNSW), so `SearchQuality` is not applicable there; Mobile and Tauri are HNSW-backed via core defaults and expose the presets (`crates/velesdb-mobile/src/types.rs` `SearchQuality`, `crates/tauri-plugin-velesdb/src/helpers.rs` `parse_search_quality`).

| Component | Status | Notes |
|-----------|--------|-------|
| Core | ✅ (source of truth) | |
| Server | ✅ | |
| Python | ✅ | |
| WASM | N/A | Brute-force only, no HNSW index; the mode string is still validated by core's `parse_search_mode` |
| Mobile | ✅ | `SearchQuality` enum mapped to core (`search_with_quality`) |
| CLI | ✅ | |
| TS SDK | ✅ | |
| Tauri | ✅ | `search_quality` param + `hnsw_m`/`hnsw_ef_construction` at creation |
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
| `SearchQuality` | 9/10 | N/A for WASM only (brute-force, no HNSW) |
| `CollectionType` | 9/10 | Haystack `Vector` only by protocol; all native crates full |

---

## Recently Landed (2026-10-29)

- **The Python integrations call core instead of copying it.** `velesdb-python` now exposes `velesdb.hash_id` (`velesdb_core::hash_id`), `velesdb.metric_higher_is_better` (`DistanceMetric::higher_is_better`) and `velesdb.validate_search_quality` (`api_types::parse_search_mode`). The MIT `velesdb_common` package dropped its FNV-1a copy (`stable_hash_id(algorithm="fnv1a")` delegates, and raises `ImportError` on a wheel older than 7.2.0) and its search-quality grammar (it delegates and wraps core's `ValueError` as `SecurityError`; an older wheel leaves the check to search time). The SHA-256 default of `stable_hash_id` and its documented divergence from core ([KNOWN_LIMITATIONS #12](./KNOWN_LIMITATIONS.md#12-string--u64-point-id-hashing-differs-across-components)) are unchanged.
- **LlamaIndex score direction fixed for Jaccard.** `VelesDBVectorStore` treated Jaccard, a similarity in core, as a distance and inverted its ranking through `1 / (1 + s)`. The direction now comes from the binding via `velesdb_common.scores.score_to_similarity` (offline fallback pinned to the binding by a drift test), with an end-to-end regression test on a real Jaccard collection. Not done here: LangChain's `similarity_search_with_relevance_scores` still applies `score >= score_threshold` to raw scores, which keeps the farthest documents under `euclidean`/`hamming`; it is a separate change to what that method returns.
- **`SearchConfigOptions.default_mode` maps through core's serde names** instead of a hand-written table in `crates/velesdb-python/src/options/search.rs`, so a `SearchMode` variant core adds is accepted from Python as it is from a `[search]` TOML section.
- **Fusion strategy names are single-sourced in core.** `velesdb_core::FusionStrategy::from_name(name, FusionParams)` is the one reading of `rrf`, `average`/`avg`, `maximum`/`max`, `weighted`, `relative_score`/`rsf` (case-insensitive), with core's defaults (`DEFAULT_RRF_K`, `DEFAULT_WEIGHTED_*`, `DEFAULT_RELATIVE_SCORE_*`) and the validating constructors. The server (`/search/multi` and the hybrid `fusion` object), the CLI (`multi-search` and the REPL's `.hybrid-sparse`), Tauri and WASM's `fuse_results` delegate to it; Python exposes it as `FusionStrategy.from_name`. This removed three drifts: the CLI's `weighted` used the pre-#1545 0.5/0.3/0.2, the REPL was case-sensitive and refused `avg`, and Tauri refused `avg`/`max`. WASM's N-branch `rsf` aggregation stays local (D005); only its name parsing moved. Still on their own name tables, outside this change: the MIT `velesdb_common.fusion` builder and the TS SDK's `FUSION_STRATEGY_NAMES` (`sdks/typescript/src/backends/wasm-search.ts`).
- **WASM's search-mode grammar is core's.** `SearchQuality` and `api_types::parse_search_mode` no longer need `persistence`, so `velesdb-wasm` validates `search_with_quality` modes with core's parser. Its local copy in `parsing.rs` is gone, and an equivalence test pins the decision on a shared table.

## Recently Landed (2026-06-14)

- **WASM fusion now delegates 4/5 strategies to core.** `average`/`maximum`/`weighted`/`rrf` map onto `velesdb_core::FusionStrategy::fuse` (ranking identical to core, pinned by an equivalence test); `relative_score`/`rsf` stays WASM-local by design because its N-branch equal-weight semantics differ from core's two-branch dense+sparse weighted sum. See the RSF/Weighted note above.
- **Executor-level conformance now exists for core.** `conformance/velesql_executor_cases.json` + `crates/velesdb-core/tests/velesql_executor_conformance.rs` assert result rows/counts/ordering (not just that a query parses). Extended to the WASM and CLI executors on 2026-06-20 (action item 2 — now done).
- **Scalar `ORDER BY` + `LIMIT` correctness bug fixed.** A scalar (non-`similarity()`) `ORDER BY <col> ... LIMIT k` previously truncated to `k` in storage order *before* sorting; it now fetches the full matching set so the sort precedes truncation, restoring the [KNOWN_LIMITATIONS #9](./KNOWN_LIMITATIONS.md#9-bounded-query-result-materialization) bounded==unbounded guarantee. The `similarity()`-ordered HNSW fast path was untouched, so recall is unaffected. (Surfaced by the new executor conformance net above.)
- **Point-ID hashing single-sourced for Haystack.** The Haystack `DocumentStore` now imports the canonical `velesdb_common.ids.stable_hash_id` instead of a bit-identical forked copy (behaviour-preserving; removes a re-implemented hash from an MIT package). The intentional remaining divergence — `velesdb-migrate`'s distinct `stable_point_id` — is documented in [KNOWN_LIMITATIONS #12](./KNOWN_LIMITATIONS.md#12-string--u64-point-id-hashing-differs-across-components).
- **(2026-07-23) FNV-1a stable-hash dedup, plus an opt-in Python alignment path (#1542).** `velesdb-core` now exports a public `hash_id_bytes`; `velesdb-memory::id::stable_id[_bytes]` and `velesdb-migrate::pipeline::fnv1a64` delegate to it instead of each re-declaring the FNV-1a offset/prime constants — behaviour-preserving (golden-vector regression tests pin the pre-refactor output for ASCII and multi-byte UTF-8). This removes the duplication risk flagged by the parity audit but does **not** change the ecosystem-level divergence: `velesdb_common.ids.stable_hash_id`'s SHA-256 default is intentionally unchanged (flipping it would orphan every string ID already stored via LangChain/LlamaIndex/Haystack). It gains an opt-in `algorithm="fnv1a"` parameter, pure-Python and MIT-licensed, verified byte-for-byte against core's published golden vectors, for callers that need a Python-derived ID to agree with core/`velesdb-migrate`'s FNV-1a for the same string. `velesdb-migrate`'s numeric-preserve fast-path is now explicitly documented as a third, deliberately distinct semantics. Full contract in [KNOWN_LIMITATIONS #12](./KNOWN_LIMITATIONS.md#12-string--u64-point-id-hashing-differs-across-components).

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
9. **Memory-surface gaps (2026-08 audit).**
   - **Autograph exposure**: the decoupled autograph path
     (`MemoryService::spawn_autograph_worker`) is spawned only by the MCP
     server/daemon (`McpServer::new`); the Python, Node, and TS-SDK bindings
     expose no way to enable autograph at all — neither the inline mode nor
     the worker. Follow-up decision: expose it, or record the daemon-only
     scoping as deliberate.
   - **LangGraph toolkit omissions**: `make_memory_tools` exposes ten tools
     and deliberately omits `entity`, `unrelate`, and every context-compiler
     tool (plus `list_working_contexts`, tracked above). Presumed by-design
     toolkit scoping — to be confirmed and recorded, not inferred.
   - **Mobile**: `velesdb-mobile` ships only a thin semantic-memory slice
     (`VelesSemanticMemory`) plus its own RAM-only `MobileGraphStore`; no doc
     records whether full memory-wedge parity on mobile is wanted or
     declined. Open decision.
   - **REST relation/TTL divergences from the memory wedge**
     (documented pending a decision, not silently): REST `relate`
     (`POST /collections/{name}/relations`,
     `crates/velesdb-server/src/handlers/points/relations.rs:114`) is
     non-idempotent — a fresh auto-assigned edge id per POST, and no
     self-loop refusal — where the wedge derives the edge id from
     `(from, relation, to)` and refuses `from == to`; REST `unrelate`
     (`DELETE .../relations/{edge_id}`, `relations.rs:204`) answers 404 on an
     absent edge where the wedge replays safely with `found: false`; REST TTL
     (`PATCH .../points/{id}/ttl`, `relations.rs:290`) accepts
     `ttl_seconds: 0` as expire-now where the wedge refuses `Some(0)`
     (`ZeroTtl`).
10. **Read-path gate (observer) parity (audit F-5.4).** The 3.10.0 read-path gate — `velesdb_core::observer` + `database::gated_search` / `authorize_read` (scope AND-composed with the caller filter, fail-closed on non-filterable paths) — is wired into **`velesdb-server`** (search/match/graph handlers) and **`velesdb-python`** (`.search()` gating). `tauri-plugin-velesdb` ships a **notify-only** observer (no deny). **`velesdb-mobile`** now ships a deny-capable observer (`open_with_observer` + `MobileObserver`/`MobileAccessDecision`, `crates/velesdb-mobile/src/lib.rs:145`). **`velesdb-node`** (memory-only, out of scope by design) and **`velesdb-wasm`** expose **no** observer, so the gate is inactive there. This is a **parity gap, not a vulnerability**: the core contract is fail-open *only when no observer is registered* (no policy ⇒ no denial to enforce), so an ungated binding simply has no governance layer to bypass. Governance-sensitive deployments must use the server or Python surface. Wiring an observer hook into mobile/wasm is a follow-up; node is intentionally excluded (it never touches the gated core `Collection`).
11. **Core documentation gaps (2026-10 parity audit).** ✅ **Documented (2026-10-29).** Core surfaces that had no usage documentation now have it, in the guide where a user of that surface looks:
    - [CORE_OPERATIONS_API.md](../guides/CORE_OPERATIONS_API.md) (new): the observer port (`open_with_observer`, the DDL / DML / read hooks, `AccessDecision` / `AccessScope` / `QueryAccessContext`), governed reads (`gated_search` with every `GatedRead` variant, `authorize_read`), guard-rails (`QueryLimits`, `Database::update_guardrails`, `CircuitBreaker`, `RateLimiter`), validation and stable ids (`validate_*`, `MAX_COLLECTION_NAME_LENGTH`, `MIN`/`MAX_DIMENSION`, `hash_id`, `hash_id_bytes`, `hash_edge_id`), index introspection (`list_indexes`, `indexes_memory_usage`, `order_by_index_advice`), `apply_advanced_config`, auto-reindex detection, the filter-strategy decision and planner, the standalone `ColumnStore` with its filters and TTLs, and the metrics types including `graph_metrics_prometheus` / `match_metrics_prometheus` and every retrieval metric.
    - [api-reference.md](./api-reference.md): every `/metrics` family the server writes, `velesdb_graph_*` and `velesdb_match_*` included; the `/search` request fields the table omitted (`mode`, `ef_search`, `timeout_ms`, the sparse and fusion fields) and what `/search/batch` does with them; the real `/guardrails` defaults, which the example misstated (`memory_limit_bytes` 1 GiB and `rate_limit_qps` 100, for 100 MiB and 100 000).
    - [CORE_COLLECTIONS_AND_METRICS.md](../guides/CORE_COLLECTIONS_AND_METRICS.md): `AnyCollection`'s shared operations and `is_*` / `as_*` / `into_*` access; [SEARCH_MODES.md](../guides/SEARCH_MODES.md): the `api_types` mode parser and ef bounds; [AGENT_MEMORY.md](../guides/AGENT_MEMORY.md): the `*_with_quality` searches, incoming and bounded relations, and relations on episodic and procedural memory; [CORE_API_MAP.md](../guides/CORE_API_MAP.md): the import paths of all of the above.

    Writing them down surfaced behaviour that is now documented as it is, and is a code follow-up rather than a doc one: `PUT /guardrails` / `update_guardrails` does not reach the rate limiter or the circuit breaker, nor collections opened afterwards; `deferred_indexing` and `async_index_builder` set by `apply_advanced_config` (and by `POST /collections`) take effect at the next open; several `/metrics` families are declared but never fed (`velesdb_documents_total`, `velesdb_index_size_bytes`, `velesdb_active_connections`, `velesdb_queries_by_type{type="graph"}`, `velesdb_limits_exceeded_total`, `velesdb_rate_limit_requests_total`, `velesdb_cache_collision_fallback_total`, `velesdb_traversal_*`, `velesdb_match_guardrail_hits_total`, `velesdb_match_similarity_queries_total`), and `velesdb_match_latency_seconds` has no `_sum` / `_count`; `IndexInfo::memory_bytes` is always `0`.
