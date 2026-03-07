# Roadmap: VelesDB v1.5 Open-Core

## Overview

VelesDB v1.5 adds four interdependent engine subsystems (Product Quantization, Sparse Vectors, Query Plan Cache, Streaming Inserts) on top of the production-quality v1.4.1 engine. The delivery sequence is dependency-driven: quality issues that would destabilize engine work are resolved first, then engine subsystems are built in the order their internal dependencies demand (PQ before Sparse, Cache before Streaming), and finally SDK parity, documentation, and release artifacts close the milestone. Every phase delivers a coherent, verifiable capability — no horizontal layer slicing.

## Phases

- [ ] **Phase 1: Quality Baseline & Security** - Fix blocking bugs, migrate bincode RUSTSEC advisory, harden CI gates
- [x] **Phase 2: PQ Core Engine** - Production-quality PQ training (k-means++), ADC SIMD kernels, no API surface change (completed 2026-03-06)
- [ ] **Phase 3: PQ Integration** - VelesQL TRAIN command, QuantizationConfig PQ variant, recall benchmark suite
- [x] **Phase 4: Sparse Vector Engine** - WeightedPostingList inverted index, sparse persistence, ANN inner-product search (completed 2026-03-06)
- [x] **Phase 5: Sparse Integration** - Hybrid dense+sparse RRF, VelesQL SPARSE_NEAR grammar, REST endpoints, u32 term_id (completed 2026-03-06)
- [x] **Phase 6: Query Plan Cache** - Two-level CompiledPlanCache, write_generation invalidation, lifecycle hooks, metrics (completed 2026-03-07)
- [x] **Phase 7: Streaming Inserts** - StreamIngester channel, micro-batches, delta buffer, searchable-immediately guarantee (completed 2026-03-07)
- [ ] **Phase 8: SDK Parity** - Python, TypeScript, WASM, Mobile, LangChain, LlamaIndex, Tauri updated to v1.5 API
- [ ] **Phase 9: Documentation** - README v1.5, rustdoc, OpenAPI spec, migration guide, benchmarks, changelog
- [ ] **Phase 10: Release Readiness** - Version bump 1.5.0, crates.io publish, PyPI wheels, npm packages, GitHub release

## Phase Details

### Phase 1: Quality Baseline & Security
**Goal**: The codebase is free of known security advisories and blocking bugs, and CI enforces quality gates that will hold across all subsequent engine work
**Depends on**: Nothing (first phase)
**Requirements**: QUAL-01, QUAL-02, QUAL-03, QUAL-04, QUAL-05, QUAL-06, QUAL-07
**Success Criteria** (what must be TRUE):
  1. `cargo audit` fails CI when a real advisory is present — the `|| true` escape hatch is gone and a `deny.toml` allowlist documents any accepted exceptions
  2. A VelesQL query using multi-alias FROM returns correct results — BUG-8 no longer produces silently wrong output
  3. Calling `ProductQuantizer::train()` with an invalid dimension config returns a typed `VelesError`, not a server crash
  4. k-means++ initialization is used for PQ codebook training — sequential deterministic init is replaced
  5. Criterion baseline v1.5 is recorded in `benchmarks/baseline.json` and the 15% regression threshold is enforced across all 35+ suites
**Plans**: 4 plans

Plans:
- [ ] 01-01-PLAN.md — Migrate bincode to postcard + harden CI audit (QUAL-01, QUAL-05)
- [ ] 01-02-PLAN.md — PQ error handling + k-means++ initialization (QUAL-03, QUAL-04)
- [ ] 01-03-PLAN.md — Fix BUG-8 multi-alias FROM in VelesQL (QUAL-02)
- [ ] 01-04-PLAN.md — Criterion baseline + coverage enforcement (QUAL-06, QUAL-07)

### Phase 2: PQ Core Engine
**Goal**: Product quantization is production-quality internally — k-means++ codebook training, ADC SIMD lookup-table kernels, OPQ pre-rotation, RaBitQ quantization, and GPU-accelerated training are all implemented with no changes to any public-facing API
**Depends on**: Phase 1
**Requirements**: PQ-01, PQ-02, PQ-03, PQ-04, PQ-ADV-01, QUANT-ADV-01
**Success Criteria** (what must be TRUE):
  1. A codebook trained on 10K real embedding vectors with m=8 k=256 produces no degenerate centroids (no two centroids closer than 1e-6) and achieves recall@10 >= 85% in the property test harness
  2. ADC distance computation uses the SIMD dispatch path (AVX2/NEON/scalar fallback) and the lookup table fits in L1 cache (m x k x 4 bytes <= 8KB for m=8 k=256)
  3. OPQ pre-rotation can be enabled via config flag — when enabled on a clustered dataset, recall@10 improves by at least 3% over standard PQ on the same data
  4. Rescore oversampling is active by default — silent recall collapse is not possible when using PQ-compressed HNSW search
  5. RaBitQ encodes vectors as binary codes with 32x compression and achieves recall@10 >= 85% on clustered test data
  6. GPU k-means assignment accelerates training when dataset exceeds FLOP threshold, with silent CPU fallback
**Plans**: 4 plans

Plans:
- [ ] 02-01-PLAN.md — PQ training hardening + codebook persistence (PQ-01)
- [ ] 02-02-PLAN.md — ADC SIMD + LUT precompute + rescore config (PQ-02, PQ-04)
- [ ] 02-03-PLAN.md — RaBitQ quantization strategy (PQ-ADV-01)
- [ ] 02-04-PLAN.md — OPQ pre-rotation + GPU k-means acceleration (PQ-03, QUANT-ADV-01)

### Phase 3: PQ Integration
**Goal**: Users can configure and use product quantization through VelesQL and the standard collection config — PQ behaves as a first-class peer to SQ8 and Binary quantization
**Depends on**: Phase 2
**Requirements**: PQ-05, PQ-06, PQ-07
**Success Criteria** (what must be TRUE):
  1. A user can run `TRAIN QUANTIZER ON my_collection WITH (m=8, k=256)` in VelesQL and the command succeeds or returns a descriptive error — training is explicit, never automatic
  2. A collection can be created or reconfigured with `QuantizationConfig::ProductQuantization` without breaking existing collections that use SQ8 or Binary — no deserialization errors on existing data
  3. The `pq_recall` Criterion suite runs and its recall@10 >= 92% threshold for m=8 is recorded in `benchmarks/baseline.json` alongside existing benchmarks
**Plans**: 3 plans

Plans:
- [ ] 03-01-PLAN.md — QuantizationConfig PQ variant + collection config (PQ-06)
- [x] 03-02-PLAN.md — VelesQL TRAIN QUANTIZER grammar/AST/parser (PQ-05) (completed 2026-03-06)
- [ ] 03-03-PLAN.md — TRAIN executor wiring + recall benchmark (PQ-07)

### Phase 4: Sparse Vector Engine
**Goal**: VelesDB can store and search sparse vectors internally — the WeightedPostingList inverted index is built, persisted to disk, and queryable via inner-product ANN search
**Depends on**: Phase 3
**Requirements**: SPARSE-01, SPARSE-02, SPARSE-03
**Success Criteria** (what must be TRUE):
  1. A sparse vector in `{term_id: u32 -> weight: f32}` format can be upserted into a collection and the data is recoverable after process restart (sparse.idx survives restart)
  2. A sparse ANN search by inner product returns results with correct relative ordering on a corpus of 1K SPLADE-format documents
  3. The sparse index write path uses segment-level isolation (single RwLock mutable buffer + immutable frozen segments) — a concurrent insert benchmark at 16 threads shows no single-lock contention bottleneck (research: segment isolation outperforms term_id sharding for SPLADE workloads where a single insert touches ~120 posting lists)
**Plans**: 3 plans

Plans:
- [x] 04-01-PLAN.md — SparseVector types + SparseInvertedIndex segment isolation + Point integration (SPARSE-01) (completed 2026-03-06)
- [ ] 04-02-PLAN.md — MaxScore DAAT search + linear scan fallback + Criterion benchmark (SPARSE-03)
- [ ] 04-03-PLAN.md — WAL persistence + compaction + Collection/Database integration (SPARSE-02)

### Phase 5: Sparse Integration
**Goal**: Hybrid dense+sparse search is fully accessible through VelesQL, the REST API, and the existing RRF fusion path — users can query sparse and hybrid from day one
**Depends on**: Phase 4
**Requirements**: SPARSE-04, SPARSE-05, SPARSE-06, SPARSE-07
**Success Criteria** (what must be TRUE):
  1. A hybrid dense+sparse search query via VelesQL `SPARSE_NEAR` clause returns fused RRF results that include both dense HNSW hits and sparse posting-list hits — the existing `fusion::rrf_merge()` is used unmodified
  2. The VelesQL grammar accepts and parses `vector SPARSE_NEAR $sv` — the conformance test suite in `conformance/velesql_parser_cases.json` includes sparse cases that pass in all crates
  3. The REST API accepts `sparse_vector` in upsert payloads and exposes a sparse search endpoint — both are documented in the OpenAPI spec
  4. All term IDs are stored as u32 — a test corpus with term_id values up to 2^32-1 is inserted and retrieved correctly
**Plans**: 4 plans

Plans:
- [ ] 05-01-PLAN.md — Named sparse vectors data model + CRUD wiring + u32 boundary test (SPARSE-04, SPARSE-07)
- [ ] 05-02-PLAN.md — VelesQL SPARSE_NEAR grammar + parser + AST + conformance (SPARSE-05)
- [ ] 05-03-PLAN.md — RSF fusion + filtered sparse search + hybrid executor wiring (SPARSE-04)
- [ ] 05-04-PLAN.md — REST API sparse endpoints + WASM sparse bindings (SPARSE-06, SPARSE-07)

### Phase 6: Query Plan Cache
**Goal**: Repeated identical VelesQL queries are served from a compiled plan cache — latency on cache hits is measurably lower and the cache invalidates correctly on all write paths
**Depends on**: Phase 5
**Requirements**: CACHE-01, CACHE-02, CACHE-03, CACHE-04
**Success Criteria** (what must be TRUE):
  1. Running the same VelesQL query twice shows a cache hit on the second execution — `EXPLAIN` output includes `cache_hit: true` and `plan_reuse_count`
  2. Inserting a document into a collection invalidates all compiled plans for that collection — the next query execution is a cache miss and re-plans correctly
  3. Dropping or recreating a collection immediately invalidates all its cached plans — no stale plan survives collection drop/recreate
  4. Cache metrics (hit rate, miss rate, evictions) are visible at the `/metrics` Prometheus endpoint after several queries
**Plans**: 2 plans

Plans:
- [x] 06-01-PLAN.md — PlanKey/CompiledPlan types + write_generation + schema_version + cache on Database (CACHE-01, CACHE-02, CACHE-03) (completed 2026-03-07)
- [x] 06-02-PLAN.md — Cache integration in execute_query + EXPLAIN cache_hit + invalidation tests + Prometheus metrics (CACHE-01, CACHE-02, CACHE-03, CACHE-04) (completed 2026-03-07)

### Phase 7: Streaming Inserts
**Goal**: Clients can insert vectors continuously without forced batching — inserts are immediately searchable, backpressure is signaled correctly, and the HNSW rebuild race is eliminated
**Depends on**: Phase 6
**Requirements**: STREAM-01, STREAM-02, STREAM-03, STREAM-04, STREAM-05
**Success Criteria** (what must be TRUE):
  1. A client inserting vectors via the streaming endpoint receives HTTP 429 when the internal buffer is full — backpressure is observable and not a silent block
  2. A vector inserted through the streaming path is findable in a search query issued immediately after the insert returns — the searchable-immediately guarantee holds
  3. 100 vectors inserted during an ongoing HNSW rebuild are all present in search results after the rebuild completes — no inserts are silently lost during auto-reindex
  4. The streaming insert code is excluded from the WASM build via `#[cfg(feature = "persistence")]` — `cargo build -p velesdb-wasm --no-default-features` compiles without error
**Plans**: 3 plans

Plans:
- [x] 07-01-PLAN.md — StreamIngester + StreamingConfig + WriteMode + DeltaBuffer stub + Collection wiring (STREAM-01, STREAM-02, STREAM-05)
- [x] 07-02-PLAN.md — DeltaBuffer search merge + rebuild coordination + searchable-immediately (STREAM-03, STREAM-04)
- [x] 07-03-PLAN.md — REST stream/insert endpoint + WASM exclusion verification (STREAM-01, STREAM-05)

### Phase 8: SDK Parity
**Goal**: All SDK and integration surfaces expose the full v1.5 API — Python, TypeScript, WASM, Mobile, LangChain, LlamaIndex, and the Tauri plugin all support sparse vectors, PQ config, and streaming inserts
**Depends on**: Phase 7
**Requirements**: SDK-01, SDK-02, SDK-03, SDK-04, SDK-05, SDK-06, SDK-07
**Success Criteria** (what must be TRUE):
  1. A Python script can call `sparse_search()`, `pq_train()`, and `stream_insert()` on a VelesDB collection using the published velesdb-python package
  2. A TypeScript/Node.js script can call `sparseSearch()`, `streamInsert()`, and set PQ config via the TypeScript SDK
  3. The WASM module runs sparse search without the `persistence` feature — `cargo build -p velesdb-wasm --no-default-features` produces a working browser module that supports sparse search
  4. A LangChain VectorStore or LlamaIndex integration example demonstrates hybrid dense+sparse search using the v1.5 API
**Plans**: 4 plans

Plans:
- [ ] 08-01-PLAN.md — Python SDK sparse/PQ/streaming (SDK-01)
- [x] 08-02-PLAN.md — TypeScript SDK + WASM sparse wiring (SDK-02, SDK-03) (completed 2026-03-07)
- [ ] 08-03-PLAN.md — Mobile UniFFI + Tauri plugin v1.5 parity (SDK-04, SDK-07)
- [ ] 08-04-PLAN.md — LangChain + LlamaIndex example integrations (SDK-05, SDK-06)

### Phase 9: Documentation
**Goal**: Every public-facing artifact (README, rustdoc, OpenAPI, migration guide, benchmarks, changelog) accurately reflects the v1.5 release and contains no stale v1.4 claims
**Depends on**: Phase 8
**Requirements**: DOC-01, DOC-02, DOC-03, DOC-04, DOC-05, DOC-06
**Success Criteria** (what must be TRUE):
  1. The README shows real v1.5 benchmark numbers (PQ recall, sparse search latency, streaming throughput) — no placeholder values from v1.4
  2. Every public type and function in `velesdb-core` has a rustdoc comment — `cargo doc --no-deps` produces zero "missing documentation" warnings for public items
  3. The OpenAPI spec includes all new v1.5 endpoints (sparse upsert, sparse search, stream insert) and passes the OpenAPI round-trip CI check
  4. The migration guide covers all breaking changes: `QuantizationConfig` wire format, VelesQL `SPARSE_NEAR` syntax, bincode -> postcard on-disk format
**Plans**: 4 plans

Plans:
- [x] 09-01-PLAN.md — README v1.5 update + CHANGELOG completion (DOC-01, DOC-06) (completed 2026-03-07)
- [ ] 09-02-PLAN.md — Rustdoc link fixes + OpenAPI spec generation (DOC-02, DOC-03)
- [ ] 09-03-PLAN.md — Migration guide + VelesQL spec + guides updates (DOC-04)
- [ ] 09-04-PLAN.md — Benchmarks v1.5 with real Criterion numbers (DOC-05)

### Phase 10: Release Readiness
**Goal**: VelesDB v1.5.0 is published across all package registries with validated cross-platform artifacts and an automated CI release matrix that prevents broken publishes
**Depends on**: Phase 9
**Requirements**: REL-01, REL-02, REL-03, REL-04, REL-05
**Success Criteria** (what must be TRUE):
  1. All workspace crates are at version 1.5.0 and `velesdb-core` is successfully published to crates.io before dependent crates — no dependency-order publish failures
  2. PyPI wheels are available for linux-x86_64, linux-aarch64, macos-arm64, and windows-x86_64 — a fresh `pip install velesdb` on each platform installs without errors
  3. `@wiscale/velesdb` and `@wiscale/velesdb-wasm` at version 1.5.0 are published on npm — `npm install @wiscale/velesdb` works
  4. The GitHub Release includes binary artifacts for Linux, macOS ARM, macOS Intel, and Windows, plus structured release notes listing all v1.5 features and breaking changes
**Plans**: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7 -> 8 -> 9 -> 10

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Quality Baseline & Security | 3/4 | In Progress|  |
| 2. PQ Core Engine | 4/4 | Complete   | 2026-03-06 |
| 3. PQ Integration | 2/3 | In Progress|  |
| 4. Sparse Vector Engine | 3/3 | Complete   | 2026-03-06 |
| 5. Sparse Integration | 4/4 | Complete   | 2026-03-06 |
| 6. Query Plan Cache | 2/2 | Complete   | 2026-03-07 |
| 7. Streaming Inserts | 3/3 | Complete   | 2026-03-07 |
| 8. SDK Parity | 3/4 | In Progress|  |
| 9. Documentation | 1/4 | In Progress |  |
| 10. Release Readiness | 0/TBD | Not started | - |
