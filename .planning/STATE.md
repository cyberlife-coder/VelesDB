# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** v3-ecosystem-alignment  
**Phase:** 1 of 7 (WASM Rebinding) + v3-08 (WASM Feature Parity)  
**Plan:** 5 of 5 (v3-08 complete)  
**Status:** v3-08 complete — Phase 1 ready to execute  
**Completed Milestones:**  
- v1-refactoring (2026-02-06 → 2026-02-08) — 7 phases, 29 plans  
- v2-core-trust (2026-02-08) — 4 phases, 10 plans  
- v4-verify-promise (2026-02-08 → 2026-02-09) — 8 phases, 30 plans  

---

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components (server, WASM, SDK, integrations) are bindings/wrappers.  
> Zero reimplemented logic. Zero duplicated code.

## Codebase Status

- **3,350+ tests** passing, 0 failures (workspace)
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
- **v3-specific:** Zero reimplementation — if WASM needs a feature, add it to core first
- **v3-specific:** Backward compatible SDK API — same function signatures, correct behavior

---

## Milestone v3: Ecosystem Alignment

**Goal:** Align entire VelesDB ecosystem with velesdb-core. Every external component becomes a proper binding/wrapper with zero reimplemented logic. All demos and examples updated to reflect v4 changes.  
**Source:** 30 findings (22 Devil's Advocate + 8 ecosystem audit)  
**Project:** `.planning/milestones/v3-ecosystem-alignment/PROJECT.md`  
**Roadmap:** `.planning/milestones/v3-ecosystem-alignment/ROADMAP.md`

### Progress

```
Phase 1  ░░░░░░░░░░  0%   WASM Rebinding (5 plans) 🚨
Phase 2  ░░░░░░░░░░  0%   Server Binding & Security 🚨
Phase 3  ░░░░░░░░░░  0%   Python Common + Integr.  🐛
Phase 4  ░░░░░░░░░░  0%   TypeScript SDK Fixes     🐛
Phase 5  ░░░░░░░░░░  0%   Demos & Examples Update  📝
Phase 6  ░░░░░░░░░░  0%   Tauri Plugin Audit       🐛
Phase 7  ░░░░░░░░░░  0%   GPU + Ecosystem CI       ⚠️
```

### Phases

| Phase | Name | Requirements | Priority | Status |
|-------|------|-------------|----------|--------|
| 1 | WASM Rebinding | ECO-01,02,06,07,16,17 | 🚨 Architecture | ⬜ Pending |
| 2 | Server Binding & Security | ECO-03,04,05,14 | 🚨 Security | ⬜ Pending |
| 3 | Python Common + Integrations | ECO-11,12,13,18,19,20 | 🐛 DRY + Quality | ⬜ Pending |
| 4 | TypeScript SDK Fixes | ECO-08,09,10,15 | 🐛 Contracts | ⬜ Pending |
| 5 | Demos & Examples Update | ECO-23→28,30 | 📝 User Experience | ⬜ Pending |
| 6 | Tauri Plugin Audit | ECO-29 | 🐛 Completeness | ⬜ Pending |
| 7 | GPU + Ecosystem CI | ECO-21,22 | ⚠️ Polish | ⬜ Pending |

**Execution:** `1 → 2 → 3 → 4 → 5`

### Deferred Requirements (from v1/v2)
- **TEST-05**: Fuzz testing expansion
- **TEST-06**: Loom concurrency testing expansion
- **TEST-07**: Benchmark regression testing in CI
- **DOCS-05**: Architecture Decision Records (ADRs)
- **DOCS-06**: Migration guide for breaking changes
- **QUAL-05**: Migrate from bincode to maintained serialization library (RUSTSEC-2025-0141)

---

## Session Continuity

**Last session:** 2026-02-09  
**Stopped at:** Phase v3-08 (WASM Feature Parity) complete — 5 plans, 11 equivalence tests, Playwright browser validation

## Decisions

| Decision | Context | Date |
|----------|---------|------|
| ColumnStore extracted from persistence gate | Only `from_collection.rs` depends on persistence; rest is pure logic | 2026-02-09 |
| IndexedDB persistence via snapshot (schema+rows JSON) | Avoids exposing internal types (RoaringBitmap, StringTable) | 2026-02-09 |
| Playwright MCP for WASM browser testing | Native targets can't run wasm_bindgen; Playwright validates in real Chromium | 2026-02-09 |
| serde_wasm_bindgen Map→toObj() helper | serde_json::Map serializes as JS Map, not plain object | 2026-02-09 |

## Blockers & Concerns

- `velesdb-python` release build has pre-existing PyO3 linker errors on Windows

---

## Quick Reference

### Key File Paths
- `.planning/milestones/v3-ecosystem-alignment/PROJECT.md` — Milestone definition (30 findings)
- `.planning/milestones/v3-ecosystem-alignment/ROADMAP.md` — Roadmap (7 phases)
- `.planning/MILESTONES.md` — All milestones summary
- `.planning/milestones/` — Archived milestones (v1, v2, v4)
- `.planning/DEVIL_ADVOCATE_FINDINGS.md` — Full review findings (47 issues)
- `AGENTS.md` — Coding standards and templates

### Key Commands
```powershell
# Core quality gates
cargo fmt --all
cargo clippy -- -D warnings
cargo deny check
cargo test --workspace
cargo build --release
.\scripts\local-ci.ps1

# Ecosystem-specific
wasm-pack test --headless --chrome    # WASM (Phase 1)
npm test                               # TypeScript SDK (Phase 3)
pytest                                 # Python integrations (Phase 4)
```

*State file last updated: 2026-02-09*  
*Status: v3-08 (WASM Feature Parity) complete. Phase 1 planned — 5 plans across 5 sequential waves. Ready to execute Plan 01 (Audit).*
