# Milestone v4: Verify Promise — Completed

**Started:** 2026-02-08  
**Completed:** 2026-02-09  
**Phases:** 8  
**Plans:** 30  
**Tests at completion:** 3,339 passing (0 failures, 67 ignored)  
**Quality gates:** fmt ✅, clippy ✅, deny ✅, test ✅, release build ✅

---

## Vision

Audit-and-fix milestone that validates whether VelesDB actually delivers what its README, GitHub page, and documentation promise. Every business scenario query, every code example, every feature claim was tested end-to-end. Where gaps existed, they were implemented or documentation was corrected.

## Requirements (13 total — all satisfied)

| ID | Requirement | Phase | Status |
|----|-------------|-------|--------|
| VP-001 | MATCH WHERE operators (LIKE, BETWEEN, IN) | Phase 1 | ✅ |
| VP-002 | Scalar subquery execution | Phase 2 | ✅ |
| VP-003 | Temporal expressions in MATCH WHERE | Phase 1 | ✅ |
| VP-004 | Multi-hop MATCH chain traversal | Phase 3 | ✅ |
| VP-005 | RETURN aggregation for MATCH results | Phase 3 | ✅ |
| VP-006 | ORDER BY property in MATCH | Phase 1 | ✅ |
| VP-007 | E2E scenario test suite | Phase 4 | ✅ |
| VP-008 | README metrics & claims accuracy | Phase 5 | ✅ |
| VP-009 | Documentation cross-consistency | Phase 5 | ✅ |
| VP-010 | Cross-store query execution | Phase 6+7 | ✅ |
| VP-011 | BM25 + NEAR VelesQL integration | Phase 6 | ✅ |
| VP-012 | NEAR_FUSED execution wiring | Phase 6 | ✅ |
| VP-013 | EXPLAIN for NEAR_FUSED & cross-store | Phase 7 | ✅ |

## Phases

| Phase | Name | Plans | Tests Added | Key Deliverables |
|-------|------|-------|-------------|------------------|
| 1 | MATCH WHERE Completeness | 2 | 21 | ORDER BY in MATCH, temporal WHERE |
| 2 | Subquery Decision & Execution | 3 | 12 | Scalar subquery executor |
| 3 | Multi-hop MATCH & RETURN | 2 | 10 | Hop-by-hop chain traversal, RETURN aggregation |
| 4 | E2E Scenario Test Suite | 7 | 36 | 7 test files covering all README scenarios |
| 5 | README & Documentation Truth | 4 | — | 9 GAPs resolved, docs honest mirror |
| 6 | Unified Query & Full-Text | 4 | 47 | NEAR_FUSED, BM25+NEAR, cross-store planner |
| 7 | Cross-Store Exec & EXPLAIN | 3 | 23 | VectorFirst/Parallel strategies, EXPLAIN nodes |
| 8 | VelesQL Execution Completeness | 5 | 27 | Database::execute_query(), JOIN, UNION/INTERSECT/EXCEPT |

**Total: 30 plans, 8 phases, ~176 new tests**

## Critical Findings Resolved

| Finding | Severity | Resolution |
|---------|----------|------------|
| MATCH WHERE `_ => Ok(true)` catch-all | 🚨 Critical | Proper operator dispatch for LIKE/BETWEEN/IN |
| Subquery → Value::Null | 🚨 Critical | Full scalar subquery executor implemented |
| Multi-hop only uses first pattern | ⚠️ Major | Hop-by-hop chain traversal |
| RETURN aggregation not implemented | ⚠️ Major | OpenCypher implicit grouping |
| ORDER BY property in MATCH | ⚠️ Major | Wired into MATCH execution pipeline |
| Temporal in MATCH WHERE | ⚠️ Major | NOW()/INTERVAL resolution in where_eval |
| RIGHT/FULL JOIN "falls back to INNER" | ⚠️ Docs | Corrected to UnsupportedFeature error |
| README test count stale | ⚠️ Docs | 3,100+ → 3,300+ |
| pgvector "700x faster" unverifiable | ⚠️ Docs | Qualified with actual numbers |

## Key Technical Decisions

- **Subquery:** Implemented full execution (not "document as unsupported")
- **Multi-hop:** Hop-by-hop chain execution with per-hop relationship filtering
- **RETURN aggregation:** OpenCypher implicit grouping (non-aggregated = grouping keys)
- **Cross-store:** VectorFirst and Parallel execution strategies via QueryPlanner
- **JOIN execution:** Database::execute_query() bridge for cross-collection operations
- **Documentation:** Honest mirror principle — no aspirational claims without labels

## Archive Contents

```
v4-verify-promise/
├── STATE.md                           # Final state snapshot
├── MILESTONE.md                       # This file
├── v4-01-match-where-completeness/    # Phase 1
├── v4-02-subquery-decision/           # Phase 2
├── v4-03-multi-hop-match-return/      # Phase 3
├── v4-04-e2e-scenario-tests/          # Phase 4
├── v4-05-readme-documentation-truth/  # Phase 5
├── v4-06-unified-query-fulltext/      # Phase 6
├── v4-07-cross-store-exec-explain/    # Phase 7
└── v4-08-velesql-execution-completeness/ # Phase 8
```
