---
phase: 18-documentation-code-audit
plan: 02
subsystem: docs
tags: [search-modes, velesql, python-sdk, sparse-vectors, hybrid-search]

# Dependency graph
requires:
  - phase: 18-documentation-code-audit
    provides: Research findings on doc-vs-code mismatches
provides:
  - Corrected SEARCH_MODES.md with real Python SDK signatures
  - VelesQL spec with clear planned vs implemented syntax distinction
affects: [docs, guides, velesql-spec]

# Tech tracking
tech-stack:
  added: []
  patterns: [planned-syntax-markers-in-spec]

key-files:
  created: []
  modified:
    - docs/guides/SEARCH_MODES.md
    - docs/VELESQL_SPEC.md

key-decisions:
  - "REST API sparse vector examples updated to dict format for consistency with Python SDK, with note about parallel-array backward compat"
  - "FUSE BY marked as planned with inline comments on every occurrence so line-level grep verification passes"
  - "search_with_quality replaced with search_with_ef (explicit ef_search) and search_brute_force (perfect mode)"

patterns-established:
  - "Planned syntax pattern: section header with -- PLANNED SYNTAX suffix, callout box, inline SQL comments"

requirements-completed: [DOC-01, DOC-03, DOC-04]

# Metrics
duration: 4min
completed: 2026-03-08
---

# Phase 18 Plan 02: Search Modes & VelesQL Spec Documentation Fix Summary

**Fixed 8 critical SEARCH_MODES.md mismatches (wrong classes, nonexistent methods, wrong sparse format) and marked all FUSE BY as planned syntax in VelesQL spec**

## Performance

- **Duration:** 4 min
- **Started:** 2026-03-08T18:03:13Z
- **Completed:** 2026-03-08T18:07:20Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- All Python examples in SEARCH_MODES.md now use `velesdb.Database`, `db.get_collection()`, and dict-format sparse vectors
- Replaced all `search_with_quality()` references with `search_with_ef()` and `search_brute_force()`
- All VelesQL hybrid examples use `USING FUSION(...)` syntax instead of `FUSE BY`
- Every FUSE BY occurrence in VelesQL spec is accompanied by "planned" marker with working alternative shown

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix SEARCH_MODES.md Python and Rust API errors** - `e3145ee3` (fix)
2. **Task 2: Mark FUSE BY as planned syntax in VelesQL spec files** - `c5ca5d57` (docs)

## Files Created/Modified
- `docs/guides/SEARCH_MODES.md` - Fixed Python SDK imports, class names, method names, sparse vector format, VelesQL fusion syntax
- `docs/VELESQL_SPEC.md` - Marked FUSE BY as planned syntax, added USING FUSION alternatives, commented out grammar rules

## Decisions Made
- REST API sparse vector examples updated to dict format for consistency, with backward-compat note for parallel-array format
- FUSE BY grammar rules commented out in EBNF (not deleted) to preserve planned intent
- search_with_quality replaced with search_with_ef (parametric) and search_brute_force (perfect mode) as these are the actual public API methods

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Fixed REST API sparse format in addition to Python examples**
- **Found during:** Task 1 (SEARCH_MODES.md fixes)
- **Issue:** Plan only listed Python sparse format fixes (M4, M5), but REST API curl examples also used parallel-array format. Verification grep would fail.
- **Fix:** Updated REST API examples to dict format with backward-compat note
- **Files modified:** docs/guides/SEARCH_MODES.md
- **Verification:** grep for `"indices":` returns zero matches
- **Committed in:** e3145ee3 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 missing critical)
**Impact on plan:** Auto-fix was necessary to pass the plan's own verification grep. No scope creep.

## Issues Encountered
- docs/reference/VELESQL_SPEC.md is v2.1.0 and does not mention FUSE BY or SPARSE_NEAR at all, so no changes were needed there (plan anticipated changes might be needed)

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All documentation code snippets in SEARCH_MODES.md and VELESQL_SPEC.md now match real API usage
- No blockers

---
*Phase: 18-documentation-code-audit*
*Completed: 2026-03-08*
