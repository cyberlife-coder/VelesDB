# Phase 5: README & Documentation Truth — Context

**Captured:** 2026-02-09 (updated post-Phase 8)

## Vision

The README.md must be an **honest mirror** of what velesdb-core actually delivers today. Every feature claim, code example, API endpoint, and performance number must be verifiable against the codebase. Aspirational features must be clearly marked as such.

## User Experience

A developer evaluating VelesDB should:
- Copy-paste any code example and have it work
- Trust every performance number (with clear context/hardware)
- Know exactly which VelesQL features execute end-to-end vs parse-only
- See real API endpoints that respond correctly
- Understand ecosystem maturity (what's production-ready vs preview)

## Essentials

Things that MUST be true:
- Every VelesQL query shown in README must parse AND execute
- Every REST endpoint listed must be routed and accessible
- Performance numbers must not conflict with each other
- GAPs between claims and reality must be documented separately
- Business scenarios must use only working query patterns

## Boundaries

Things to explicitly AVOID:
- Do NOT remove features that parse but don't execute — document them honestly as "Parser support only"
- Do NOT inflate metrics or round up
- Do NOT show aspirational queries without marking them as "Planned"
- Do NOT claim ecosystem components are published when they may not be

## Implementation Notes

Specific technical preferences:
- Create a standalone GAPS.md for tracking documentation vs reality issues
- Update the PHASE.md file if it doesn't exist
- The feature truth document should be authoritative and referenceable

## Open Questions

Things to decide during planning:
- Should business scenarios be rewritten to use only working patterns, or kept aspirational with clear labels?
  - **Partially answered by Phase 8:** JOIN and compound queries NOW WORK, so more scenarios are valid. Remaining limitations: correlated cross-collection subqueries in MATCH, temporal interval precision.
- ~~Should the `/query/explain` endpoint be routed (tiny fix) or removed from docs?~~
  - **✅ RESOLVED by Phase 8 Plan 08-04:** `/query/explain` is now routed and working.

## Phase 8 Impact Summary

Phase 8 (VelesQL Execution Completeness) resolved 3 of the original FALSE gaps:
- **GAP-1 (JOIN):** INNER + LEFT JOIN now fully executed via `Database::execute_query()`
- **GAP-2 (UNION/INTERSECT/EXCEPT):** All 4 set operators now executed
- **GAP-3 (/query/explain):** Route added to server
- README already updated: parser-only labels removed, `/query/explain` in API table
- FEATURE_TRUTH.md updated for new execution status
- **Remaining GAPs:** 4 (GraphService in-memory), 5 (perf conflicts), 6 (business scenario limits), 7 (API table incomplete), 8 (ecosystem claims), 9 (test counts)

---
*This context informs planning. The planner will honor these preferences.*
