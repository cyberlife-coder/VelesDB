# Milestones

## v4-verify-promise — In Progress

**Started:** 2026-02-08
**Status:** Gap analysis complete. Ready to plan Phase 1.
**Phases:** 5
**Estimated effort:** ~40-50h

### Summary

Audit-and-fix milestone that validates whether VelesDB actually delivers what its README, GitHub page, and website promise. Every business scenario query, every code example, every feature claim is tested end-to-end. Where gaps exist, they are implemented or documentation is corrected.

### Critical Findings

| Finding | Severity | Impact |
|---------|----------|--------|
| MATCH WHERE `_ => Ok(true)` catch-all | 🚨 Critical | LIKE/BETWEEN/IN silently pass — wrong results |
| Subquery → Value::Null | 🚨 Critical | ALL 4 business scenario queries broken |
| Multi-hop only uses first pattern | ⚠️ Major | Multi-relationship MATCH incomplete |
| RETURN aggregation not implemented | ⚠️ Major | Healthcare scenario broken |
| ORDER BY property in MATCH → error | ⚠️ Major | AI Agent Memory scenario broken |
| Temporal in MATCH WHERE not wired | ⚠️ Major | Fraud detection scenario broken |

### Phases

| Phase | Name | Requirements | Estimate |
|-------|------|-------------|----------|
| 1 | MATCH WHERE Completeness | VP-001, VP-003, VP-006 | 8-10h |
| 2 | Subquery Decision & Execution | VP-002 | 10-12h |
| 3 | Multi-hop MATCH & RETURN | VP-004, VP-005 | 10-12h |
| 4 | E2E Scenario Test Suite | VP-007 | 8-10h |
| 5 | README & Documentation Truth | VP-008, VP-009 | 4-6h |

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
