# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** v2-core-trust (Phase 1 done, Phase 2 next)  
**Next Milestone:** v3-ecosystem-alignment  
**Previous Milestone:** v1-refactoring (completed 2026-02-08)  

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

### Status: Phase 1 done. Phase 2 (Critical Correctness) next.

| Phase | Status | Tasks | Findings | Estimate | Priority |
|-------|--------|-------|----------|----------|----------|
| 0 - Merge & Tag v1 | ✅ Done | 1 | — | 15 min | 🔒 Prerequisite |
| 1 - CI Safety Net | ✅ Done | 4 | CI-01→04 | 15min | 🛡️ Infrastructure |
| 2 - Critical Correctness | ⬜ Pending | 7 | C-01→03, D-09 | 8-10h | 🚨 Wrong Results |
| 3 - Core Engine Bugs | ⬜ Pending | 7 | B-01,02,04→06, D-08, M-03 | 6-8h | 🐛 Correctness |
| 4 - Perf, Storage, Cleanup | ⬜ Pending | 9 | D-01→07, M-01→02 | 8-10h | ⚠️ Optimization |

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
- Old broken `score_fusion/mod.rs` FusionStrategy enum → DELETE in Phase 2
- `GpuTrigramAccelerator` → DELETE, rename to `TrigramAccelerator`
- BFS overflow: stop inserting, don't `clear()` visited set
- ORDER BY property → return error, not silent no-op
- DualPrecision default → use int8 when quantizer trained
- Tests multi-thread in CI (remove `--test-threads=1`)

## Milestone v3: Ecosystem Alignment (22 findings — bindings/wrappers)

### Status: Blocked by v2 completion

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
*Status: Phase 1 complete (4/4 tasks). Phase 2 (Critical Correctness — GPU + Fusion) next.*
