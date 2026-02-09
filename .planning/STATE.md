# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** v4-verify-promise (Phases 1-8 ✅ complete)  
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

### Codebase Status (post-refactoring, post-correctness)
- **~3,339 tests** passing, 0 failures (workspace)
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

### Status: Phases 1-4 ✅, Phase 6-7 ✅ complete. Phase 5 planned.

| Phase | Status | Tasks | Requirements | Estimate | Priority |
|-------|--------|-------|-------------|----------|----------|
| 1 - MATCH WHERE Completeness | ✅ Done (2/2 plans) | 17+4 tests | VP-001 ✅, VP-003 ✅, VP-006 ✅ | 2-3h | 🚨 Silent incorrect results |
| 2 - Subquery Decision & Execution | ✅ Done | 12 tests | VP-002 | 10-12h | 🚨 All README scenarios broken |
| 3 - Multi-hop MATCH & RETURN | ✅ Done | 10 tests | VP-004, VP-005 | 10-12h | ⚠️ Business scenarios |
| 4 - E2E Scenario Test Suite | ✅ Done (7/7 plans, 36 tests) | 36 | VP-007 | 8-10h | 🛡️ Regression prevention |
| 5 - README & Documentation Truth | ✅ Done (4/4 plans) | 6 GAPs resolved | VP-008 ✅, VP-009 ✅ | 4-6h | 📝 Trust & credibility |
| 6 - Unified Query & Full-Text | ✅ Done (4 plans) | 47 new tests | VP-010 ✅, VP-011 ✅, VP-012 ✅ | 14-19h | 🚨 Cross-store + NEAR_FUSED |
| 7 - Cross-Store Exec & EXPLAIN | ✅ Done (3 plans) | 23 new tests | VP-010 ✅, VP-013 ✅ | 8-12h | 🚨 VectorFirst/Parallel + EXPLAIN |

**Total:** ~50 tasks | ~50-65h
**Execution:** `1 → 2 → 3 → 4 → 5 → 6`

### Critical Findings

| Finding | Severity | Location | Impact |
|---------|----------|----------|--------|
| MATCH WHERE `_ => Ok(true)` catch-all | 🚨 Critical | `where_eval.rs:69` | LIKE/BETWEEN/IN silently pass in MATCH |
| ~~Subquery → Value::Null~~ | ✅ Fixed | `subquery.rs` + `mod.rs` | Resolved by VP-002 Phase 2 |
| ~~Multi-hop only uses first pattern~~ | ✅ Fixed | `match_exec/mod.rs` | Resolved by VP-004 Phase 3 Plan 1 |
| ~~RETURN aggregation not implemented~~ | ✅ Fixed | `match_exec/return_agg.rs` | Resolved by VP-005 Phase 3 Plan 2 |
| ~~ORDER BY property in MATCH → silently ignored~~ | ✅ Fixed | `match_exec/mod.rs` + `similarity.rs` | Resolved by VP-006 Plan 01-01 (2026-02-08) |
| ~~Temporal in MATCH WHERE → silent false~~ | ✅ Fixed | `where_eval.rs:resolve_where_param` | Resolved by VP-003 Plan 01-02 (2026-02-08) |

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
- **Wave 1:** ✅ 04-01 Test Infrastructure & Hero Query (completed 2026-02-08)
- **Wave 2:** ✅ 04-02 SELECT Domain, ✅ 04-03 Metrics & Fusion, ✅ 04-04 Simple MATCH, ✅ 04-05 Complex MATCH, ✅ 04-06 Cross-Store & VelesQL
- **Wave 3:** ✅ 04-07 Quality Gates & Integration (completed 2026-02-09)
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

**Phase 1 ✅ complete (2/2 plans):**
- 01-01: ✅ Wire ORDER BY into MATCH execution pipeline (VP-006) — completed 2026-02-08
- 01-02: ✅ Wire Temporal resolution into MATCH WHERE comparison (VP-003) — completed 2026-02-08
- Plans dir: `.planning/phases/v4-01-match-where-completeness/`

**Phase 5 ✅ complete (4/4 plans, 2 waves) — executed 2026-02-09:**
- **Wave 1:** ✅ 05-01 VelesQL Spec & Feature Matrix Truth (RIGHT/FULL JOIN fix, JOIN USING added, Database::execute_query() docs), ✅ 05-02 README Metrics (test count 3,100+ → 3,300+, TS SDK name fix, server 25+ endpoints)
- **Wave 2:** ✅ 05-03 README Query & Scenario Accuracy (MATCH syntax fix, API table +2 endpoints, pseudocode label), ✅ 05-04 Website Claims Audit (pgvector claim qualified, illustrative disclaimer, ROI context)
- **All 9 GAPs resolved:** GAP-1/2/3 by Phase 8, GAP-4/5/6/7/8/9 by Phase 5
- Cross-document consistency verified: README ↔ VELESQL_SPEC ↔ FEATURE_TRUTH
- Plans dir: `.planning/phases/v4-05-readme-documentation-truth/`

**Phase 7 ✅ complete (3 plans, 1 wave) — executed 2026-02-10:**
- **Deferred from Phase 6:** VectorFirst/Parallel cross-store execution + EXPLAIN nodes for NEAR_FUSED and cross-store
- ✅ **07-01:** Cross-Store VectorFirst & Parallel Execution (8 tests) — `cross_store_exec.rs`, `QueryPlanner` wired into `execute_query()`
- ✅ **07-02:** EXPLAIN Nodes for NEAR_FUSED & Cross-Store (8 tests) — `FusedSearch` + `CrossStoreSearch` PlanNodes, rendering, JSON
- ✅ **07-03:** Integration & Quality Gates (7 E2E tests) — all gates pass (fmt, clippy, deny, test, release build)
- New files: `cross_store_exec.rs`, `cross_store_exec_tests.rs`, `phase7_integration.rs`
- Plans dir: `.planning/phases/v4-07-cross-store-exec-explain/`

**Phase 6 ✅ complete (4 plans, 2 waves) — executed 2026-02-10:**
- **Key finding confirmed:** BM25, Trigram, Fusion, Planner all EXISTED — gaps were wiring, not implementation
- **Wave 1:** ✅ 06-01 NEAR_FUSED Execution Wiring (VP-012, 13 tests), ✅ 06-02 BM25+NEAR VelesQL Integration (VP-011, 7 tests)
- **Wave 2:** ✅ 06-03 Cross-Store Planner & Dispatch (VP-010, 18 tests + dispatch extraction), ✅ 06-04 Benchmarks & Quality Gates (9 E2E tests + near_fused_bench)
- **Deferred:** Cross-store VectorFirst/Parallel execution strategies, EXPLAIN support for new query paths
- Plans dir: `.planning/phases/v4-06-unified-query-fulltext/`
- New files: `dispatch.rs`, `bm25_integration_tests.rs`, `cross_store_tests.rs`, `near_fused_bench.rs`, `phase6_integration.rs`
- mod.rs reduced from 451 → 212 lines; hybrid.rs dead_code removed

**Phase 8 ✅ complete (5/5 plans) — VelesQL Execution Completeness:**
- ✅ **08-01:** Database-Level Query Executor + ColumnStore Bridge — `Database::execute_query()`, compound queries (UNION/INTERSECT/EXCEPT)
- ✅ **08-02:** JOIN Execution Integration — LEFT JOIN support, RIGHT/FULL → `UnsupportedFeature` error, 9 new tests (7 Database-level + 2 error tests), `execute_join()` returns `Result`
- ✅ **08-03:** Compound Query Execution — 7 Database-level integration tests (UNION, UNION ALL, INTERSECT, EXCEPT, same-collection, not-found error, order-by). Executor+wiring already existed from 08-01; gap was integration tests.
- ✅ **08-04:** /query/explain Route & Server Integration — `/query` handler wired to `Database::execute_query()` for cross-collection JOIN + compound support. `/query/explain` route already existed.
- ✅ **08-05:** E2E Tests & Documentation Update — 11 E2E tests (`e2e_join.rs`, `e2e_compound.rs`), README parser-only labels removed, `/query/explain` added to API table, GAPS.md GAP-1/2/3 marked RESOLVED.
- Plans dir: `.planning/phases/v4-08-velesql-execution-completeness/`
- New files: `database_query_tests.rs`, `tests/e2e_join.rs`, `tests/e2e_compound.rs`

*State file last updated: 2026-02-09*  
*Status: All phases complete (1-8). 3,339 workspace tests passing. Quality gates all green. Phase 5 executed — all 9 GAPs resolved, README is an honest mirror of codebase.*
