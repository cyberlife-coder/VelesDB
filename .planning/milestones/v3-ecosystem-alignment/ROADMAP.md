# VelesDB v3 — Ecosystem Alignment Roadmap (Expanded)

**Version:** 2.1  
**Created:** 2026-02-09  
**Milestone:** v3-ecosystem-alignment  
**Previous Milestones:** v1-refactoring, v2-core-trust, v4-verify-promise  
**Total Phases:** 8 (+ 3 decimal)  
**Findings covered:** 30 (22 Devil's Advocate + 8 ecosystem audit) + feature parity audit  
**Source:** `DEVIL_ADVOCATE_FINDINGS.md` + v3 ecosystem audit

---

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components are bindings/wrappers. Zero reimplemented logic.  
> If a feature doesn't exist in core, it doesn't exist anywhere.

---

## Phase 1: WASM Rebinding 🚨

**Goal:** Replace the WASM VectorStore reimplementation with proper bindings to velesdb-core. This is the #1 architectural problem — 21 source files that reimplement the engine.

**Requirements:** ECO-01, ECO-02, ECO-06, ECO-07, ECO-16, ECO-17  
**Depends on:** Nothing  
**WASM constraint:** `velesdb-core` with `default-features = false` (no persistence/tokio/mmap/rayon). Need `persistence` feature flag for conditional compilation. See memory: "WASM Build: mio/tokio/memmap2/rayon Incompatibility".

### Tasks

1. **Audit WASM → Core mapping** — Map every WASM public function to its velesdb-core equivalent; identify gaps where core needs new non-persistence exports
2. **Core API surface for WASM** — Expose in-memory-only Collection/Search/Graph APIs behind a `wasm` or `no-persistence` feature flag (key: no `memmap2`, no `rayon`, no `tokio`)
3. **Replace VectorStore** — Delete `store_*.rs`, `vector_ops.rs`, `simd.rs`, `quantization.rs`, `filter.rs`, `fusion.rs`, `text_search.rs`. Bind to core's Collection/Index/Search
4. **Replace GraphStore** — Delete `graph.rs`, `graph_persistence.rs`, `graph_worker.rs`. Bind to core's EdgeStore/graph traversal
5. **Replace Agent** — Delete `agent.rs`. Bind to core's agent memory if exists, or mark as deferred
6. **Fix clippy suppression** — Remove blanket `#![allow(clippy::pedantic)]` + `#![allow(clippy::nursery)]` + 14 others. Keep only wasm_bindgen-necessary allows (e.g., `needless_pass_by_value` for JsValue)
7. **Tests:** WASM search matches core search for identical data; all storage modes (Full, SQ8, Binary) work correctly; graph traversal matches core; VelesQL execution matches core

### Success Criteria

- [ ] WASM VectorStore delegates to velesdb-core (zero own search/insert/distance logic)
- [ ] Graph operations use core's EdgeStore (zero own BFS/DFS)
- [ ] Clippy pedantic enabled with targeted allows only
- [ ] Search results **identical** between WASM and core for same data + same query
- [ ] `wasm-pack test` passes
- [ ] WASM binary size ≤ current (or justified if larger due to core inclusion)

---

## Phase 2: Server Binding & Security 🚨

**Goal:** Server becomes a thin HTTP layer over velesdb-core. Add authentication and runtime safety.

**Requirements:** ECO-03, ECO-04, ECO-05, ECO-14  
**Depends on:** Nothing (parallel with Phase 1)

### Tasks

1. **Authentication** — API key middleware (`Authorization: Bearer <key>` + `VELESDB_API_KEY` env var). Optional (disabled if env var not set) for dev mode
2. **Unify graph** — Delete `GraphService` in-memory store. Server graph handlers bind to core's Database EdgeStore
3. **`spawn_blocking`** — Wrap ALL CPU-intensive handlers: search, query, traversal, batch upsert. Only health/list can stay async
4. **Rate limiting** — `tower-governor` or `tower::limit::RateLimit`, configurable per IP. Default: 100 req/s
5. **Remove duplicate BFS/DFS** — Server calls core's `Database::traverse()`, not its own implementation
6. **Tests:** Auth rejection (401); concurrent search doesn't block; rate limit returns 429; graph results match core; spawn_blocking verified

### Success Criteria

- [ ] Unauthenticated request → 401 (when API key configured)
- [ ] Zero reimplemented graph/traversal logic in server
- [ ] CPU handlers wrapped in `spawn_blocking`
- [ ] Rate limiting active and returns 429
- [ ] All server tests pass

---

## Phase 3: Python Common + Integrations 🐛

**Goal:** Extract shared Python library, fix LangChain/LlamaIndex bugs, eliminate 80% code duplication.

**Requirements:** ECO-11, ECO-12, ECO-13, ECO-18, ECO-19, ECO-20  
**Depends on:** Nothing (parallel with Phases 1-2)

### Tasks

1. **Extract `velesdb-python-common`** — New package in `integrations/common/` with shared code:
   - `_stable_hash_id()` (currently duplicated)
   - `security.py` (currently duplicated — 315/317 lines near-identical)
   - Result parsing helpers
   - Payload building helpers
2. **Fix `storage_mode` dead code** — Pass `storage_mode` to `create_collection()` in both integrations (ECO-13/BEG-02)
3. **Fix ID generation** — Remove `self._next_id = 1` counter. Use hash-based IDs from `_stable_hash_id()` exclusively (ECO-11/I-01)
4. **Fix `velesql()` validation** — Add `validate_query()` call in LlamaIndex's `velesql()` method (ECO-12/I-02)
5. **Factor `add_texts_bulk`** — DRY with `add_texts` via shared internal `_upsert_documents()` method (ECO-19/BEG-03)
6. **Fix security validation** — `validate_path` must sandbox to a `base_directory` parameter. Reject absolute paths outside sandbox. `validate_query` must do basic SQL injection checks beyond null/length (ECO-20/BEG-04)
7. **Update both integrations** to import from `velesdb-python-common`
8. **Tests:** storage_mode SQ8 creates correct collection; no ID collision across instances; velesql injection blocked; path traversal blocked; shared library unit tests

### Success Criteria

- [ ] `velesdb-python-common` package exists and is imported by both integrations
- [ ] Zero duplicated code between LangChain and LlamaIndex
- [ ] `storage_mode` actually applied to collection creation
- [ ] ID collision impossible (hash-based only)
- [ ] Path traversal attacks blocked by sandbox
- [ ] `pytest` passes for both integrations

---

## Phase 4: TypeScript SDK Fixes 🐛

**Goal:** SDK correctly maps server responses, handles concurrent init, and aligns with server API.

**Requirements:** ECO-08, ECO-09, ECO-10, ECO-15  
**Depends on:** Phase 2 (server API must be stable first)

### Tasks

1. **`search()` response** — Extract `.results` from server response (like `textSearch`/`hybridSearch` already do) (ECO-08/T-01)
2. **`listCollections()`** — Unwrap `{ collections: [...] }` wrapper, return `Collection[]` (ECO-09/T-02)
3. **`query()` collection param** — Either remove the unused parameter or document it's extracted from VelesQL `FROM` clause (ECO-15/T-03)
4. **`init()` race guard** — Promise-based singleton lock for concurrent callers (ECO-10/BEG-07)
5. **Update WASM backend** — If Phase 1 changes the WASM API surface, update `wasm-backend.ts` accordingly
6. **Tests:** search returns `SearchResult[]`; listCollections returns `Collection[]`; concurrent init is safe; query works with new server auth

### Success Criteria

- [ ] `search()` → `SearchResult[]` (not wrapped object)
- [ ] `listCollections()` → `Collection[]` (not wrapped object)
- [ ] No init race condition (verified with concurrent test)
- [ ] `npm test` passes
- [ ] SDK works with auth-enabled server (Phase 2)

---

## Phase 4.1: TypeScript SDK Documentation & Examples Completeness 📝

**Goal:** Fix all documentation gaps and example deficiencies found during SDK audit. Every public method documented in README, comprehensive examples for all features, and streaming traversal route coverage.

**Inserted:** 2026-02-11 — Post-audit found 6 undocumented methods, 1 missing route, 6 missing example files  
**Depends on:** Phase 4 (SDK code fixes must be done first)

### Plans

1. **03.1-01: Fix README.md Documentation Gaps** — Add matchQuery(), explain(), SelectBuilder, searchBatch(), createMetadataCollection(), SearchMode/efSearch to README (~1h)
2. **03.1-02: Add Comprehensive SDK Examples** — 6 new example files (graph, match, builders, explain, fusion, indexes) + update hybrid_queries.ts to use real SDK calls (~2h)
3. **03.1-03: Implement Streaming Traversal + Tests** — Add streamTraverseGraph() for SSE route (the only 1/21 server route not covered by SDK) (~1.5h)

### Success Criteria

- [ ] All public SDK methods documented in README (currently 6 missing)
- [ ] 7 example files (6 new + 1 updated) covering all SDK features
- [ ] `streamTraverseGraph()` implemented for REST backend (SSE)
- [ ] 21/21 server routes covered by SDK (currently 20/21)
- [ ] `npm test` passes
- [ ] README under 600 lines

---

## Phase 4.2: Python SDK Parity Fixes 🚨

**Goal:** Fix the structural mismatch between Python integrations and the Python SDK. Integrations call 10+ methods on `Collection` that don't exist in the Rust PyO3 bindings. Add graph methods to SDK, fix wrong method names in integrations, guard unimplemented features, update READMEs.

**Inserted:** 2026-02-11 — Post-audit revealed phantom methods that pass mocked tests but crash in production  
**Depends on:** Phase 4.1 (integration-side methods must exist first)

### Plans

1. **04.2-01: SDK Graph Methods on Collection (Rust)** — Add `add_edge`, `get_edges`, `get_edges_by_label`, `traverse`, `get_node_degree` to `crates/velesdb-python/src/collection.rs` via PyO3, delegating to core's EdgeStore (~4h)
2. **04.2-02: Fix Integration Method Names + NotImplementedError Guards** — Rename `create_index`→`create_property_index`, `delete_index`→`drop_index`. Guard `match_query`/`explain` with `NotImplementedError`. Fix `add_node` in GraphLoader. Fix `list_collections` docstring (~2h)
3. **04.2-03: SDK Contract Tests** — Create `test_sdk_contract.py` verifying every `self._collection.xxx()` call uses a method that actually exists on `velesdb.Collection` via `hasattr()` (~2h)
4. **04.2-04: README Updates (Both Integrations)** — Document all features, add SDK method parity table, document known limitations (~3h)
5. **04.2-05: Remaining Gaps Backlog** — Document deferred items (AgentMemory LlamaIndex, ProceduralMemory, VelesQL Parser exposure) in backlog (~1h)

### Success Criteria

- [ ] All `self._collection.xxx()` calls in integrations correspond to real SDK methods
- [ ] `cargo test --workspace` passes (Rust SDK changes)
- [ ] `pytest` passes for both integrations (zero regressions)
- [ ] `match_query()`/`explain()` raise `NotImplementedError` (not `AttributeError`)
- [ ] `create_index`/`delete_index` renamed to correct SDK names
- [ ] READMEs document all public methods
- [ ] Deferred gaps documented in backlog

---

## Phase 5: Demos & Examples Update 📝

**Goal:** Every demo compiles, every example runs, every user-facing artifact reflects the current API.

**Requirements:** ECO-23, ECO-24, ECO-25, ECO-26, ECO-27, ECO-28, ECO-30  
**Depends on:** Phases 1-4 (SDKs must be correct before updating demos)

### Tasks

1. **`rag-pdf-demo`** — Update to use Python SDK instead of raw httpx. Verify all VelesQL queries use v4 syntax. Update `pyproject.toml` dependencies. Ensure `python -m pytest` passes
2. **`tauri-rag-app`** — Verify builds with current `tauri-plugin-velesdb`. Update version to match core. Test basic RAG flow
3. **Python examples** (`examples/python/`) — Verify all 6 files use correct VelesQL syntax (NEAR, NEAR_FUSED, MATCH, subqueries). Update imports if PyO3 SDK API changed
4. **Rust examples** — `cargo build` for `ecommerce_recommendation`, `mini_recommender`, `rust/multimodel_search`. Fix any API breakage
5. **WASM browser demo** — Update `index.html` to use new WASM API from Phase 1
6. **`examples/README.md`** — Update API table with all v4 endpoints (NEAR_FUSED, cross-store, EXPLAIN). Update VelesQL examples
7. **Version alignment** — All `package.json`, `pyproject.toml`, `Cargo.toml` at consistent version
8. **Smoke test all demos** — Each demo must work end-to-end from a fresh clone

### Success Criteria

- [ ] `rag-pdf-demo`: `pytest` passes, uses Python SDK
- [ ] `tauri-rag-app`: `cargo tauri build` succeeds
- [ ] All 6 Python examples: run without error against velesdb-server
- [ ] All Rust examples: `cargo build` succeeds
- [ ] WASM browser demo: works in Chrome
- [ ] All versions aligned across ecosystem
- [ ] `examples/README.md` reflects current API

---

## Phase 6: Tauri Plugin Audit 🐛

**Goal:** Verify tauri-plugin-velesdb is aligned with core, uses proper bindings, and supports the tauri-rag-app demo.

**Requirements:** ECO-29  
**Depends on:** Phase 1 (core API surface changes may affect plugin)

### Tasks

1. **Audit commands** — Map every Tauri command in `commands.rs` + `commands_graph.rs` to its core equivalent
2. **Verify types** — `types.rs` (13KB) must match core types, not redefine them
3. **Verify graph commands** — `commands_graph.rs` must use core's EdgeStore, not its own
4. **Error handling** — `error.rs` must map core errors properly
5. **Tests:** Run existing `commands_tests.rs`, verify all pass against current core

### Success Criteria

- [ ] All Tauri commands delegate to core (no reimplemented logic)
- [ ] Types match core types
- [ ] `commands_tests.rs` passes
- [ ] `tauri-rag-app` demo works end-to-end (Phase 5)

---

## Phase 7: GPU Extras + Ecosystem CI ⚠️

**Goal:** Complete GPU distance metric coverage and ensure all ecosystem components have CI.

**Requirements:** ECO-21, ECO-22  
**Depends on:** All previous phases (ecosystem must be stable)

### Tasks

1. **Hamming/Jaccard GPU shaders** — WGSL compute shaders + pipeline wiring in `gpu_backend.rs`
2. **GPU equivalence tests** — All 5 metrics (Cosine, Euclidean, DotProduct, Hamming, Jaccard) have GPU vs CPU equivalence tests
3. **Python CI** — Both LangChain and LlamaIndex `pytest` in GitHub Actions, blocking on failure
4. **TypeScript SDK CI** — `npm test` in GitHub Actions
5. **WASM CI** — `wasm-pack test --headless --chrome` in GitHub Actions
6. **Tauri CI** — At minimum `cargo check --package tauri-plugin-velesdb`

### Success Criteria

- [ ] All 5 distance metrics have real GPU pipelines (not CPU fallbacks)
- [ ] GPU vs CPU equivalence tests pass for all metrics
- [ ] Python, TS, WASM, Tauri tests all run in CI
- [ ] CI blocks on any ecosystem test failure
- [ ] CI badge green

---

## Progress Tracker

| Phase | Status | Requirements | Priority |
|-------|--------|-------------|----------|
| 1 - WASM Rebinding | ✅ Done (5/5) | ECO-01,02,06,07,16,17 | 🚨 Architecture |
| 2 - Server Binding & Security | ✅ Done (5/5) | ECO-03,04,05,14 | 🚨 Security |
| 2.1 - Server Documentation | ✅ Done | — | � Docs |
| 3 - TypeScript SDK Fixes | ✅ Done (5/5) | ECO-08,09,10,15 | 🐛 Contracts |
| 3.1 - TS SDK Docs & Examples | ✅ Done (3/3) | Audit gaps (6 docs, 6 examples, 1 route) | 📝 Completeness |
| 4 - Python Integrations | ✅ Done (3/3) | ECO-11,12,13,18,19,20 | 🐛 DRY + Quality |
| 4.1 - Python Feature Parity | ⬜ Pending | Audit: 10 missing features | 🚨 Completeness |
| 4.2 - Python SDK Parity Fixes | ⬜ Pending | Audit: phantom methods, wrong names, docs | 🚨 Production Safety |
| 5 - Demos & Examples Update | ⬜ Pending | ECO-23→28,30 | 📝 User Experience |
| 6 - Tauri Plugin Audit | ⬜ Pending | ECO-29 | 🐛 Completeness |
| 7 - GPU Extras + Ecosystem CI | ⬜ Pending | ECO-21,22 | ⚠️ Polish |
| 8 - WASM Feature Parity | ✅ Done (5/5) | — | 🚨 Architecture |

**Execution:** `1 → 2 → 3 → 4 → 4.1 → 4.2 → 5 → 6 → 7 → 8`  
**Findings covered:** 30/30 + audit gaps + feature parity audit  
*Last updated: 2026-02-11*

---

## Quality Gates (per phase)

```powershell
# Core (always — every phase)
cargo fmt --all --check
cargo clippy -- -D warnings
cargo deny check
cargo test --workspace
cargo build --release

# Ecosystem (phase-specific)
wasm-pack test --headless --chrome        # Phase 1 (WASM)
npm test                                   # Phase 4 (SDK)
pytest integrations/langchain/             # Phase 3 (LangChain)
pytest integrations/llamaindex/            # Phase 3 (LlamaIndex)
cargo check --package tauri-plugin-velesdb # Phase 6 (Tauri)
```

---
*Milestone v3 — Ecosystem Alignment (Expanded). 30 findings + parity audit, 8 phases (+3 decimal), 10 components.*
