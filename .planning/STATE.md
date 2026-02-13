# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** v3-ecosystem-alignment  
**Phase:** v3-05 complete (4/4 plans done) — Demos & Examples Update  
**Status:** Phases 1, 2, 2.1, 3, 3.1, 4, 4.1, 4.2, 4.3, 5, 8 complete. Phases 6–7 remain.  
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

- **3,350+ tests** passing (Rust workspace), 0 failures
- **304 tests** passing (TypeScript SDK), 0 failures
- **Quality gates**: fmt ✅, clippy ✅, deny ✅, test ✅, release build ✅
- **112 unsafe blocks** — all documented with SAFETY comments
- **README**: Honest mirror of codebase (verified by v4 Phase 5)
- **VelesQL**: Full execution for SELECT, MATCH, JOIN (INNER/LEFT), UNION/INTERSECT/EXCEPT, subqueries, NEAR_FUSED, BM25
- **TypeScript SDK**: 21/21 server routes covered (including streamTraverseGraph SSE)

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
Phase 1    ██████████ 100%  WASM Rebinding            ✅ (5/5 plans)
Phase 2    ██████████ 100%  Server Binding & Security  ✅ (5/5 plans)
Phase 2.1  ██████████ 100%  Server Documentation       ✅ (1 SUMMARY)
Phase 3    ██████████ 100%  TypeScript SDK Fixes        ✅ (5/5 plans)
Phase 3.1  ██████████ 100%  TS SDK Docs & Examples     ✅ (3/3 plans)
Phase 4    ██████████ 100%  Python Integrations        ✅ (3/3 plans)
Phase 4.1  ██████████ 100%  Python Feature Parity      ✅ (4/4 plans)
Phase 4.2  ██████████ 100%  Python SDK Parity Fixes    ✅ (5/5 plans)
Phase 4.3  ██████████ 100%  SDK & Integration Parity   ✅ (3/3 plans)
Phase 5    ██████████ 100%  Demos & Examples Update    ✅ (4/4 plans)
Phase 6    ░░░░░░░░░░   0%  Tauri Plugin Audit         ⬜ Pending
Phase 7    ░░░░░░░░░░   0%  GPU Extras + Ecosystem CI  ⬜ Pending
Phase 8    ██████████ 100%  WASM Feature Parity         ✅ (5/5 plans)
```

### Phases

| Phase | Name | Requirements | Priority | Status |
|-------|------|-------------|----------|--------|
| 1 | WASM Rebinding | ECO-01,02,06,07,16,17 | 🚨 Architecture | ✅ Done |
| 2 | Server Binding & Security | ECO-03,04,05,14 | 🚨 Security | ✅ Done |
| 2.1 | Server Documentation | — | � Docs | ✅ Done |
| 3 | TypeScript SDK Fixes | ECO-08,09,10,15 | 🐛 Contracts | ✅ Done |
| 3.1 | TS SDK Docs & Examples | Audit gaps | 📝 Completeness | ✅ Done |
| 4 | Python Integrations | ECO-11,12,13,18,19,20 | 🐛 DRY + Quality | ✅ Done |
| 4.1 | Python Feature Parity | Audit: 10 missing features | 🚨 Completeness | ⬜ Pending |
| 4.2 | Python SDK Parity Fixes | Audit: phantom methods, wrong names, docs | 🚨 Production Safety | ✅ Done |
| 4.3 | SDK & Integration Full Parity | Audit: MATCH+EXPLAIN missing from PyO3+integrations | 🚨 Feature Parity | ✅ Done |
| 5 | Demos & Examples Update | ECO-23→28,30 | 📝 User Experience | ✅ Done |
| 6 | Tauri Plugin Audit | ECO-29 | 🐛 Completeness | ⬜ Pending |
| 7 | GPU Extras + Ecosystem CI | ECO-21,22 | ⚠️ Polish | ⬜ Pending |
| 8 | WASM Feature Parity | — | 🚨 Architecture | ✅ Done |

### Remaining Work

1. ~~**v3-04.1**: Python Feature Parity~~ ✅ Complete (4/4 plans, 324 tests pass)
2. ~~**v3-04.2**: Python SDK Parity Fixes~~ ✅ Complete (5/5 plans)
3. ~~**v3-04.3**: SDK & Integration Full Parity~~ ✅ Complete (3/3 plans)
4. ~~**v3-05**: Demos & Examples Update~~ ✅ Complete (4/4 plans, 3 commits)
5. **v3-06**: Tauri Plugin Audit — plan & execute
6. **v3-07**: GPU Extras + Ecosystem CI — plan & execute

### Deferred Requirements (from v1/v2)
- **TEST-05**: Fuzz testing expansion
- **TEST-06**: Loom concurrency testing expansion
- **TEST-07**: Benchmark regression testing in CI
- **DOCS-05**: Architecture Decision Records (ADRs)
- **DOCS-06**: Migration guide for breaking changes
- **QUAL-05**: Migrate from bincode to maintained serialization library (RUSTSEC-2025-0141)

---

## Session Continuity

**Last session:** 2026-02-13  
**Stopped at:** Phase v3-05 complete (4/4 plans). Next: /gsd-plan-phase for Phase 6 (Tauri Plugin Audit).  
**Resume file:** .planning/.continue-here

## Decisions

| Decision | Context | Date |
|----------|---------|------|
| ColumnStore extracted from persistence gate | Only `from_collection.rs` depends on persistence; rest is pure logic | 2026-02-09 |
| IndexedDB persistence via snapshot (schema+rows JSON) | Avoids exposing internal types (RoaringBitmap, StringTable) | 2026-02-09 |
| Playwright MCP for WASM browser testing | Native targets can't run wasm_bindgen; Playwright validates in real Chromium | 2026-02-09 |
| serde_wasm_bindgen Map→toObj() helper | serde_json::Map serializes as JS Map, not plain object | 2026-02-09 |
| Graph handlers bind to core Collection API | Delete server's GraphService, use Collection::traverse_bfs/dfs | 2026-02-10 |
| Auth: constant-time comparison for API keys | Prevent timing attacks on bearer token validation | 2026-02-10 |
| spawn_blocking: all CPU handlers wrapped | Only health_check stays pure async | 2026-02-10 |
| tower-governor for per-IP rate limiting | Global rate limit is insufficient; need per-IP granularity | 2026-02-10 |
| Mock data aligned with real QueryResult/QueryStats types | SDK tests must use actual interface shapes, not invented fields | 2026-02-11 |
| Unify graph in Collection (not separate GraphStore) | DX: agents use one object; server already deleted GraphService | 2026-02-11 |
| ~~match_query/explain → NotImplementedError~~ **REVERTED** | MATCH IS implemented in core (EPIC-045 US-002, 476 lines). EXPLAIN IS implemented (EPIC-046 US-004, 539 lines). Decision was wrong — Phase 4.3 fixes this. | 2026-02-12 |
| Fix integration names, don't add SDK aliases | SDK names match core; integrations should adapt | 2026-02-11 |
| Direct PyO3 bindings to core graph API | Zero reimplementation; SDK delegates to velesdb-core | 2026-02-12 |
| Inline traversal result conversion | Core's TraversalResult differs from graph module's type | 2026-02-12 |

## Blockers & Concerns

- `velesdb-python` release build has pre-existing PyO3 linker errors on Windows

---

## Quick Reference

### Key File Paths
- `.planning/milestones/v3-ecosystem-alignment/PROJECT.md` — Milestone definition (30 findings)
- `.planning/milestones/v3-ecosystem-alignment/ROADMAP.md` — Roadmap (8+ phases)
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

*State file last updated: 2026-02-13*  
*Status: Phases 1, 2, 2.1, 3, 3.1, 4, 4.1, 4.2, 4.3, 5, 8 done. Phases 6–7 remain.*
