# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** v4-verify-promise (Phase 0 — Gap analysis complete, ready to plan)  
**Previous Milestones:** v1-refactoring (completed 2026-02-08), v2-core-trust (completed)  
**Blocked Milestone:** v3-ecosystem-alignment (blocked by v4 — no point fixing bindings if core promises are broken)  

---

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components (server, WASM, SDK, integrations) are bindings/wrappers.  
> Zero reimplemented logic. Zero duplicated code.

## Project Reference

### Core Value
VelesDB is a cognitive memory engine for AI agents — Vector + Graph + Symbolique in a single local-first engine.

### Codebase Status (post-refactoring, pre-correctness)
- **3,117 tests** passing, 0 failures
- **Quality gates**: fmt ✅, clippy ✅, deny ✅, doc ✅, release build ✅
- **112 unsafe blocks** — all documented with SAFETY comments
- **47 issues found** by Devil's Advocate review (3 audit phases): 7 critical, 14 bugs, 23 design, 3 minor
- **2 findings already fixed** (C-04 RRF, B-03 Weighted) — verified by code triage

### Constraints
- Rust 1.83+ only
- All quality gates must pass: fmt, clippy, deny, test
- All unsafe code must have documented invariants
- TDD: test BEFORE code for every fix
- Martin Fowler: files >300 lines get split into modules

---

## Milestone v2: Core Trust (23 active findings — velesdb-core only)

### Status: Phase 4 complete. All plans done. Milestone v2 complete.

| Phase | Status | Tasks | Findings | Estimate | Priority |
|-------|--------|-------|----------|----------|----------|
| 0 - Merge & Tag v1 | ✅ Done | 1 | — | 15 min | 🔒 Prerequisite |
| 1 - CI Safety Net | ✅ Done | 4 | CI-01→04 | 15min | 🛡️ Infrastructure |
| 2 - Critical Correctness | ✅ Done (3/3 plans) | 7 | C-01→03, D-09 | 8-10h | 🚨 Wrong Results |
| 3 - Core Engine Bugs | ✅ Done (3/3 plans) | 7 | B-01,02,04→06, D-08, M-03 | 6-8h | 🐛 Correctness |
| 4 - Perf, Storage, Cleanup | ✅ Done (3/3 plans) | 9 | D-01→07, M-01→02 | 8-10h | ⚠️ Optimization |

**Total:** 28 tasks | 10 plans | ~25-30h  
**Execution:** `0 → 1 → 2 → 3 → 4`

### Plan Mapping

| Phase | Plans | Wave | Files |
|-------|-------|------|-------|
| 1 | 01-01: CI Pipeline Hardening | 1 | `phases/01-ci-safety-net/01-01-PLAN.md` |
| 2 | 02-01: GPU WGSL Shaders & Metric Dispatch | 1 | `phases/02-critical-correctness/02-01-PLAN.md` |
| 2 | 02-02: GPU Trigram Cleanup | 1 | `phases/02-critical-correctness/02-02-PLAN.md` |
| 2 | 02-03: Fusion Unification & ParseError | 1 | `phases/02-critical-correctness/02-03-PLAN.md` |
| 3 | 03-01: Input Validation & VelesQL Fixes | 1 | `phases/03-core-engine-bugs/03-01-PLAN.md` |
| 3 | 03-02: Graph Traversal Fixes | 1 | `phases/03-core-engine-bugs/03-02-PLAN.md` |
| 3 | 03-03: Quantization & DualPrecision Fixes | 1 | `phases/03-core-engine-bugs/03-03-PLAN.md` |
| 4 | 04-01: HNSW & Search Performance | 1 | `phases/04-perf-storage-cleanup/04-01-PLAN.md` |
| 4 | 04-02: Storage Integrity | 1 | `phases/04-perf-storage-cleanup/04-02-PLAN.md` |
| 4 | 04-03: ColumnStore Unification & Dead Code | 1 | `phases/04-perf-storage-cleanup/04-03-PLAN.md` |

### Key Decisions (v3.2)
- C-04/B-03 removed from scope (already fixed in `fusion/strategy.rs`)
- Old broken `score_fusion/mod.rs` FusionStrategy enum → ✅ DELETED (02-03)
- `ParseError::InvalidValue` (E007) added for fusion param validation (02-03)
- `GpuTrigramAccelerator` → ✅ DONE: renamed to `TrigramAccelerator` (02-02)
- BFS overflow: stop inserting, don't `clear()` visited set
- ORDER BY property → return error, not silent no-op
- DualPrecision default → use int8 when quantizer trained
- Tests multi-thread in CI (remove `--test-threads=1`)
- D-02: HNSW `search_layer` single lock acquisition per search (not per-candidate)
- D-04: Over-fetch factor configurable via `WITH (overfetch = N)`, default 10, clamped 1-100
- D-05: WAL per-entry CRC32 for corruption detection (replay + retrieve)
- D-06: `store_batch()` with single flush for reduced I/O syscalls
- D-07: `should_create_snapshot` uses AtomicU64 instead of write lock

## Milestone v4: Verify Promise (9 requirements — promise vs reality)

### Status: Gap analysis complete. Ready to plan Phase 1.

| Phase | Status | Tasks | Requirements | Estimate | Priority |
|-------|--------|-------|-------------|----------|----------|
| 1 - MATCH WHERE Completeness | ⬜ Ready | ~8 | VP-001, VP-003, VP-006 | 8-10h | 🚨 Silent incorrect results |
| 2 - Subquery Decision & Execution | ⬜ Ready | ~5 | VP-002 | 10-12h | 🚨 All README scenarios broken |
| 3 - Multi-hop MATCH & RETURN | ⬜ Blocked by P1 | ~6 | VP-004, VP-005 | 10-12h | ⚠️ Business scenarios |
| 4 - E2E Scenario Test Suite | ⬜ Blocked by P1-3 | ~12 | VP-007 | 8-10h | 🛡️ Regression prevention |
| 5 - README & Documentation Truth | ⬜ Blocked by P4 | ~5 | VP-008, VP-009 | 4-6h | 📝 Trust & credibility |

**Total:** ~36 tasks | ~40-50h
**Execution:** `1 → 2 → 3 → 4 → 5`

### Critical Findings

| Finding | Severity | Location | Impact |
|---------|----------|----------|--------|
| MATCH WHERE `_ => Ok(true)` catch-all | 🚨 Critical | `where_eval.rs:69` | LIKE/BETWEEN/IN silently pass in MATCH |
| Subquery → Value::Null | 🚨 Critical | `conversion.rs:23-27` | ALL 4 business scenarios broken |
| Multi-hop only uses first pattern | ⚠️ Major | `match_exec/mod.rs` | Multi-relationship queries incomplete |
| RETURN aggregation not implemented | ⚠️ Major | `match_exec/similarity.rs` | Healthcare scenario broken |
| ORDER BY property in MATCH → error | ⚠️ Major | `match_exec/similarity.rs:210` | AI Agent Memory scenario broken |
| Temporal in MATCH WHERE not wired | ⚠️ Major | `where_eval.rs` | Fraud detection scenario broken |

### Key Decision Pending

**Subquery approach:** Implement execution (complex, ~10h) vs Return clear error (simple, ~2h)?
→ User decision needed before Phase 2.

---

## Milestone v3: Ecosystem Alignment (22 findings — bindings/wrappers)

### Status: Blocked by v4 — no point fixing bindings if core promises are broken

| Phase | Status | Scope | Priority |
|-------|--------|-------|----------|
| 1 - WASM Rebinding | ⬜ Blocked | BEG-01,05,06, W-01→03 | 🚨 Architecture |
| 2 - Server Binding | ⬜ Blocked | S-01→04, BEG-05 | 🚨 Security |
| 3 - SDK Fixes | ⬜ Blocked | T-01→03, BEG-07 | 🐛 Contracts |
| 4 - Python Integrations | ⬜ Blocked | I-01→03, BEG-02→04 | 🐛 Contracts |
| 5 - GPU + Ecosystem CI | ⬜ Blocked | I-04, CI-04 | ⚠️ Polish |

**Execution:** `1 → 2 → 3 → 4 → 5` (after v2 complete)

---

### Pending v2+ Requirements (deferred from v1)
- **TEST-05**: Fuzz testing expansion
- **TEST-06**: Loom concurrency testing expansion
- **TEST-07**: Benchmark regression testing in CI
- **DOCS-05**: Architecture Decision Records (ADRs)
- **DOCS-06**: Migration guide for breaking changes
- **QUAL-05**: Migrate from bincode to maintained serialization library (RUSTSEC-2025-0141)

---

## Quick Reference

### Important File Paths
- `.planning/v2-correctness/PROJECT.md` — Milestone v2 definition
- `.planning/v2-correctness/ROADMAP.md` — Milestone v2 roadmap v3.2 (0+4 phases, 28 tasks)
- `.planning/v3-ecosystem-alignment/PROJECT.md` — Milestone v3 definition
- `.planning/v3-ecosystem-alignment/ROADMAP.md` — Milestone v3 roadmap (5 phases)
- `.planning/DEVIL_ADVOCATE_FINDINGS.md` — Full review findings (47 issues)
- `AGENTS.md` — Coding standards and templates

### Key Commands
```powershell
cargo fmt --all
cargo clippy -- -D warnings
cargo deny check
cargo test --workspace
cargo build --release
.\scripts\local-ci.ps1
```

---

*State file last updated: 2026-02-08*  
*Status: Milestone v4-verify-promise created. Gap analysis complete. 6 critical/major findings. Ready to plan Phase 1.*
