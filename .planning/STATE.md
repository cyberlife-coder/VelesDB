# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** v4-verify-promise (Phase 3 ✅ — Phase 4 ready to plan)  
**Previous Milestones:** v1-refactoring (completed 2026-02-08), v2-core-trust (completed 2026-02-08)  
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

## Milestone v2: Core Trust — ✅ Completed 2026-02-08

**23/23 findings resolved** | 4 phases | 10 plans | 3,165 tests | Audit passed  
**Archive:** `.planning/milestones/v2-core-trust/` | Full details in `MILESTONES.md`

## Milestone v4: Verify Promise (9 requirements — promise vs reality)

### Status: Phase 1-3 complete. Phase 4 planned (7 plans, 3 waves).

| Phase | Status | Tasks | Requirements | Estimate | Priority |
|-------|--------|-------|-------------|----------|----------|
| 1 - MATCH WHERE Completeness | ✅ Done | 15 tests | VP-001, VP-003, VP-006 | 8-10h | 🚨 Silent incorrect results |
| 2 - Subquery Decision & Execution | ✅ Done | 12 tests | VP-002 | 10-12h | 🚨 All README scenarios broken |
| 3 - Multi-hop MATCH & RETURN | ✅ Done | 10 tests | VP-004, VP-005 | 10-12h | ⚠️ Business scenarios |
| 4 - E2E Scenario Test Suite | 📋 Planned (7 plans) | ~18 | VP-007 | 8-10h | 🛡️ Regression prevention |
| 5 - README & Documentation Truth | ⬜ Blocked by P4 | ~5 | VP-008, VP-009 | 4-6h | 📝 Trust & credibility |

**Total:** ~36 tasks | ~40-50h
**Execution:** `1 → 2 → 3 → 4 → 5`

### Critical Findings

| Finding | Severity | Location | Impact |
|---------|----------|----------|--------|
| MATCH WHERE `_ => Ok(true)` catch-all | 🚨 Critical | `where_eval.rs:69` | LIKE/BETWEEN/IN silently pass in MATCH |
| ~~Subquery → Value::Null~~ | ✅ Fixed | `subquery.rs` + `mod.rs` | Resolved by VP-002 Phase 2 |
| ~~Multi-hop only uses first pattern~~ | ✅ Fixed | `match_exec/mod.rs` | Resolved by VP-004 Phase 3 Plan 1 |
| ~~RETURN aggregation not implemented~~ | ✅ Fixed | `match_exec/return_agg.rs` | Resolved by VP-005 Phase 3 Plan 2 |
| ORDER BY property in MATCH → error | ⚠️ Major | `match_exec/similarity.rs:210` | AI Agent Memory scenario broken |
| Temporal in MATCH WHERE not wired | ⚠️ Major | `where_eval.rs` | Fraud detection scenario broken |

### Key Decisions Made

**Subquery approach:** ✅ **Implement execution** (user confirmed 2026-02-08)
- Plan 02-01: Core Scalar Subquery Executor (Wave 1)
- Plan 02-02: Wire into MATCH WHERE (Wave 2)
- Plan 02-03: Wire into SELECT WHERE + Quality Gates (Wave 2)

**Multi-hop approach:** Hop-by-hop chain execution (planned 2026-02-08)
- Plan 03-01: ✅ Multi-hop Chain Traversal + Binding-Aware WHERE (completed 2026-02-08)
- Plan 03-02: ✅ RETURN Aggregation for MATCH Results (completed 2026-02-08)

**Key technical decisions for Phase 3:**
- Hop-by-hop execution replaces single merged BFS — per-hop relationship type filtering
- Binding-aware WHERE: alias-qualified columns (`b.price`) resolved from bindings map
- RETURN aggregation uses OpenCypher implicit grouping (non-aggregated items = grouping keys)
- Single-hop path unchanged for backward compatibility

**Phase 4 plan structure (7 plans, 3 waves):**
- **Wave 1:** 04-01 Test Infrastructure & Hero Query (creates module + helpers + stubs)
- **Wave 2:** 04-02 SELECT Domain (Scenarios 1-3), 04-03 Metrics & Fusion (0b/0c), 04-04 Simple MATCH (BS1/BS4), 04-05 Complex MATCH (BS2/BS3), 04-06 Cross-Store & VelesQL (Scenario 0 + API)
- **Wave 3:** 04-07 Quality Gates & Integration
- Plans dir: `.planning/phases/v4-04-e2e-scenario-tests/`
- Test files: `tests/readme_scenarios/` (7 files)

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
*Status: Phase 4 planned (7 plans). Ready to execute with `/gsd-execute-plan`.*
