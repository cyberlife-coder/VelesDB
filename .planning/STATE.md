# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** v4-verify-promise (Phase 0 — Gap analysis complete, ready to plan)  
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
