# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** v2-core-trust (Phase 0 — merge & tag)  
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

### Constraints
- Rust 1.83+ only
- All quality gates must pass: fmt, clippy, deny, test
- All unsafe code must have documented invariants
- TDD: test BEFORE code for every fix

---

## Milestone v2: Core Trust (25 findings — velesdb-core only)

### Status: Phase 0 ready to execute

| Phase | Status | Scope | Priority |
|-------|--------|-------|----------|
| 0 - Merge & Tag v1 | ⬜ Pending | Git workflow | 🔒 Prerequisite |
| 1 - CI Safety Net | ⬜ Pending | CI-01→04 | 🛡️ Infrastructure |
| 2 - Critical Correctness | ⬜ Pending | C-01→04, B-03, D-09 | 🚨 Wrong Results |
| 3 - Core Engine Bugs | ⬜ Pending | B-01→06, D-08, M-03 | 🐛 Correctness |
| 4 - Perf, Storage, Cleanup | ⬜ Pending | D-01→07, M-01→02 | ⚠️ Optimization |

**Execution:** `0 → 1 → 2 → 3 → 4`

## Milestone v3: Ecosystem Alignment (22 findings — bindings/wrappers)

### Status: Blocked by v2 completion

| Phase | Status | Scope | Priority |
|-------|--------|-------|----------|
| 1 - WASM Rebinding | ⬜ Blocked | BEG-01,05,06, W-01→03 | 🚨 Architecture |
| 2 - Server Binding | ⬜ Blocked | S-01→04, BEG-05 | 🚨 Security |
| 3 - SDK Fixes | 🟡 In Progress (Plan 01 ✅) | T-01→03, BEG-07 | 🐛 Contracts |
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
- `.planning/v2-correctness/ROADMAP.md` — Milestone v2 roadmap (0+4 phases)
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

*State file last updated: 2026-02-09*  
*Status: v3 Phase 3 in progress (Plan 01 REST Backend ✅). Plan 02 next. v2 deferred.*
