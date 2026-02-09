# Milestones

## v3-ecosystem-alignment — In Progress

**Started:** 2026-02-09  
**Phases:** 5  
**Findings covered:** 22/47 (ecosystem)

### Summary

Align the entire VelesDB ecosystem with velesdb-core. Every external component becomes a proper binding/wrapper with zero reimplemented logic. Covers WASM rebinding, server security, SDK fixes, Python integrations, and GPU+CI.

### Phases

| Phase | Name | Findings | Priority |
|-------|------|----------|----------|
| 1 | WASM Rebinding | BEG-01,05,06, W-01→03 | 🚨 Architecture |
| 2 | Server Binding & Security | S-01→04, BEG-05 | 🚨 Security |
| 3 | TypeScript SDK Fixes | T-01→03, BEG-07 | 🐛 Contracts |
| 4 | Python Integrations | I-01→03, BEG-02→04 | 🐛 Contracts |
| 5 | GPU + Ecosystem CI | I-04, CI-04 | ⚠️ Polish |

### Archive

Details: `.planning/v3-ecosystem-alignment/`  
Phase dirs: `.planning/phases/v3-01-*` through `v3-05-*`

---

## v4-verify-promise — Completed

**Started:** 2026-02-08  
**Completed:** 2026-02-09  
**Phases:** 8  
**Plans:** 30  
**Requirements:** 13/13 satisfied (100%)

### Summary

Audit-and-fix milestone that validated whether VelesDB actually delivers what its README, GitHub page, and documentation promise. Every business scenario query, every code example, every feature claim was tested end-to-end. Where gaps existed, they were implemented or documentation was corrected. Scope expanded from 5 → 8 phases as deeper execution gaps were discovered (cross-store queries, NEAR_FUSED, JOIN/compound execution).

### Key Achievements

- **MATCH WHERE**: Fixed silent `_ => Ok(true)` catch-all — LIKE/BETWEEN/IN now execute correctly
- **Subquery Execution**: Full scalar subquery executor — all 4 README business scenarios work
- **Multi-hop MATCH**: Hop-by-hop chain traversal with binding-aware WHERE
- **RETURN Aggregation**: OpenCypher implicit grouping for MATCH results
- **NEAR_FUSED Wiring**: Multi-vector fusion search fully executable via VelesQL
- **Cross-Store Planner**: VectorFirst & Parallel execution strategies via QueryPlanner
- **Database::execute_query()**: Cross-collection JOIN + UNION/INTERSECT/EXCEPT
- **EXPLAIN**: PlanNodes for FusedSearch and CrossStoreSearch
- **Documentation Truth**: 9 GAPs resolved — README is an honest mirror of codebase

### Phases

| Phase | Name | Plans | Tests Added |
|-------|------|-------|-------------|
| 1 | MATCH WHERE Completeness | 2 | 21 |
| 2 | Subquery Decision & Execution | 3 | 12 |
| 3 | Multi-hop MATCH & RETURN | 2 | 10 |
| 4 | E2E Scenario Test Suite | 7 | 36 |
| 5 | README & Documentation Truth | 4 | — |
| 6 | Unified Query & Full-Text | 4 | 47 |
| 7 | Cross-Store Exec & EXPLAIN | 3 | 23 |
| 8 | VelesQL Execution Completeness | 5 | 27 |

### Metrics

| Metric | Value |
|--------|-------|
| Phases | 8 |
| Plans executed | 30 |
| Requirements satisfied | 13/13 (100%) |
| New tests added | ~176 |
| Tests at completion | 3,339 |
| Quality gates | 5/5 (fmt, clippy, deny, test, release build) |
| Milestone commits | ~92 |

### Archive

Full details: `.planning/milestones/v4-verify-promise/`

---

## v2-core-trust — Completed

**Started:** 2026-02-08  
**Completed:** 2026-02-08  
**Phases:** 4 (+1 prerequisite)  
**Plans:** 10  
**Audit:** ✅ Passed (23/23 findings resolved, 0 gaps)

### Summary

Implementation Truth & Correctness milestone. All findings from the Devil's Advocate Code Review (47 issues total, 23 scoped to velesdb-core). Fixed real GPU WGSL shaders, fusion disambiguation (ScoreCombineStrategy), VelesQL ParseError::InvalidValue (E007), graph traversal bugs, quantization fixes, HNSW single-lock search, WAL CRC32 integrity, and dead code cleanup.

### Key Achievements

- **GPU Truth**: Fake CPU loops replaced with real WGSL compute shaders (Euclidean + DotProduct)
- **Fusion Clarity**: Old broken `FusionStrategy` renamed to `ScoreCombineStrategy`, broken RRF variant deleted
- **Parse Safety**: `ParseError::InvalidValue` (E007) replaces silent `unwrap_or(0.0)` in fusion/MATCH parsing
- **HNSW Perf**: Single read lock per search (was per-candidate), adaptive over-fetch factor
- **Storage Integrity**: WAL per-entry CRC32, batch flush, AtomicU64 snapshot
- **CI Hardened**: PR triggers restored, security audit blocking, cargo deny, multi-threaded tests

### Metrics

| Metric | Value |
|--------|-------|
| Phases | 4 (+1 prerequisite) |
| Plans executed | 10 |
| Findings resolved | 23/23 (100%) |
| Tests passing | 3,165 |
| Quality gates | 5/5 (fmt, clippy, deny, test, release build) |
| Commits | ~22 |

### Archive

Full details: `.planning/milestones/v2-core-trust/`

---

## v1-refactoring — 2026-02-08

**Started:** 2026-02-06
**Completed:** 2026-02-08
**Phases:** 7
**Plans:** 29

### Summary

Complete code quality, safety, and maintainability refactoring of VelesDB Core. Zero breaking API changes. The codebase is now faster, cleaner, more maintainable, and production-ready.

### Key Achievements

- **SIMD Architecture**: Monolithic 2400-line `simd_native.rs` split into 14 focused modules (mod.rs now 132 lines)
- **Zero-Dispatch DistanceEngine**: Cached SIMD fn pointers eliminate per-call dispatch in HNSW hot loops (13% faster at 1536d cosine)
- **Unsafe Audit**: 112 unsafe blocks across 18 files — all documented with SAFETY comments per AGENTS.md template
- **Error Hardening**: 64 bare-string errors replaced with typed variants; 4 panic sites converted to Result
- **Dependency Cleanup**: 10 unused deps removed across 7 crates
- **Documentation**: `#![warn(missing_docs)]` enforced; 0 rustdoc warnings; README fully updated
- **Test Suite**: 3,117 tests passing (0 failures), including property-based SIMD equivalence, loom concurrency, WAL recovery edge cases

### Phases

| Phase | Name | Plans | Requirements |
|-------|------|-------|--------------|
| 1 | Foundation Fixes | 3 | RUST-01, RUST-02, RUST-03, BUG-01 |
| 2 | Unsafe Code & Testing | 3 | RUST-04, RUST-05, BUG-02, BUG-03, TEST-01 |
| 3 | Architecture & Graph | 4 | QUAL-01, QUAL-02, BUG-04, TEST-02 |
| 4 | Complexity & Errors | 9 | QUAL-03, QUAL-04, DOCS-01, DOCS-02, TEST-03 |
| 5 | Cleanup & Performance | 3 | CLEAN-01, CLEAN-02, CLEAN-03, TEST-04, PERF-01 |
| 6 | Documentation & Polish | 1 | DOCS-03, DOCS-04, PERF-02, PERF-03 |
| 7 | SIMD Tolerance & Engine | 2 | TEST-08, PERF-04 |

### Metrics

| Metric | Value |
|--------|-------|
| Phases | 7 |
| Plans executed | 29 |
| Requirements satisfied | 28/28 (100%) |
| Tests passing | 3,117 |
| Quality gates | 5/5 (fmt, clippy, deny, doc, release build) |

### Archive

Full details: `.planning/milestones/v1-refactoring/`

---
