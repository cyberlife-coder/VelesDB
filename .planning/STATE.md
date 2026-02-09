# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** None (ready for next milestone)  
**Completed Milestones:**  
- v1-refactoring (2026-02-06 → 2026-02-08) — 7 phases, 29 plans, 3,117 tests  
- v2-core-trust (2026-02-08) — 4 phases, 10 plans, 23/23 findings resolved  
- v4-verify-promise (2026-02-08 → 2026-02-09) — 8 phases, 30 plans, 13/13 requirements, ~176 new tests  
**Blocked Milestone:** v3-ecosystem-alignment (was blocked by v4 — **now unblocked**)  

---

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components (server, WASM, SDK, integrations) are bindings/wrappers.  
> Zero reimplemented logic. Zero duplicated code.

## Codebase Status

- **3,339 tests** passing, 0 failures (workspace)
- **Quality gates**: fmt ✅, clippy ✅, deny ✅, test ✅, release build ✅
- **112 unsafe blocks** — all documented with SAFETY comments
- **README**: Honest mirror of codebase (verified by v4 Phase 5)
- **VelesQL**: Full execution for SELECT, MATCH, JOIN (INNER/LEFT), UNION/INTERSECT/EXCEPT, subqueries, NEAR_FUSED, BM25

### Constraints
- Rust 1.83+ only
- All quality gates must pass: fmt, clippy, deny, test
- All unsafe code must have documented invariants
- TDD: test BEFORE code for every fix
- Martin Fowler: files >300 lines get split into modules

---

## Next Milestone Candidates

### v3-ecosystem-alignment — Now Unblocked

22 findings from Devil's Advocate review — bindings/wrappers need alignment with core.

| Phase | Scope | Priority |
|-------|-------|----------|
| 1 - WASM Rebinding | BEG-01,05,06, W-01→03 | 🚨 Architecture |
| 2 - Server Binding | S-01→04, BEG-05 | 🚨 Security |
| 3 - SDK Fixes | T-01→03, BEG-07 | 🐛 Contracts |
| 4 - Python Integrations | I-01→03, BEG-02→04 | 🐛 Contracts |
| 5 - GPU + Ecosystem CI | I-04, CI-04 | ⚠️ Polish |

**Execution:** `1 → 2 → 3 → 4 → 5`

### Deferred Requirements (from v1/v2)
- **TEST-05**: Fuzz testing expansion
- **TEST-06**: Loom concurrency testing expansion
- **TEST-07**: Benchmark regression testing in CI
- **DOCS-05**: Architecture Decision Records (ADRs)
- **DOCS-06**: Migration guide for breaking changes
- **QUAL-05**: Migrate from bincode to maintained serialization library (RUSTSEC-2025-0141)

---

## Quick Reference

### Key File Paths
- `.planning/MILESTONES.md` — All milestones summary
- `.planning/milestones/` — Archived milestones (v1, v2, v4)
- `.planning/DEVIL_ADVOCATE_FINDINGS.md` — Full review findings (47 issues)
- `.planning/v3-ecosystem-alignment/` — Next milestone candidate
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

*State file last updated: 2026-02-09*  
*Status: v4-verify-promise completed. Ready for next milestone. v3-ecosystem-alignment now unblocked.*
