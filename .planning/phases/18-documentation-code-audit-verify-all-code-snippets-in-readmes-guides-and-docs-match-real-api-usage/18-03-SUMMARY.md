---
phase: 18-documentation-code-audit
plan: 03
subsystem: docs
tags: [readme, api-reference, wasm, python, hnsw, npm]

requires:
  - phase: 18-01
    provides: root README fixes
provides:
  - Corrected REST API examples in WASM README (no /v1/ prefix, top_k param)
  - Corrected npm package name in USE_CASES.md and MULTIMODEL_QUERIES.md
  - Fixed Python README code block formatting
  - Corrected HNSW API reference (search_with_ef not search_with_quality)
  - Verified velesdb-core README exports match lib.rs
affects: []

tech-stack:
  added: []
  patterns: []

key-files:
  created: []
  modified:
    - crates/velesdb-wasm/README.md
    - docs/guides/USE_CASES.md
    - docs/guides/MULTIMODEL_QUERIES.md
    - crates/velesdb-python/README.md
    - docs/reference/NATIVE_HNSW.md

key-decisions:
  - "velesdb-core README exports (Filter, Condition, recall_at_k, precision_at_k, mrr, ndcg_at_k) verified correct against lib.rs -- no changes needed"

patterns-established: []

requirements-completed: [DOC-01, DOC-02, DOC-04]

duration: 2min
completed: 2026-03-08
---

# Phase 18 Plan 03: Fix Crate READMEs and Guide Documentation Summary

**Corrected REST routes, npm package names, Python formatting, and HNSW API references across 5 documentation files**

## Performance

- **Duration:** 2 min
- **Started:** 2026-03-08T18:03:16Z
- **Completed:** 2026-03-08T18:05:00Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- WASM README REST examples corrected: removed /v1/ prefix and replaced limit with top_k
- USE_CASES.md and MULTIMODEL_QUERIES.md now use @wiscale/velesdb-sdk instead of velesdb-client
- Python README code block formatting fixed (missing newlines before results and import)
- NATIVE_HNSW.md API table corrected: search_with_ef replaces search_with_quality
- velesdb-core README exports verified correct against actual lib.rs public API

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix WASM README REST routes and search parameters** - `38b9ec99` (fix)
2. **Task 2: Fix npm package names, Python README formatting, and NATIVE_HNSW reference** - `324acc4c` (fix)

## Files Created/Modified
- `crates/velesdb-wasm/README.md` - Fixed REST route URLs and search parameter name
- `docs/guides/USE_CASES.md` - Fixed npm package name to @wiscale/velesdb-sdk
- `docs/guides/MULTIMODEL_QUERIES.md` - Fixed npm package name to @wiscale/velesdb-sdk
- `crates/velesdb-python/README.md` - Fixed missing newlines in code blocks
- `docs/reference/NATIVE_HNSW.md` - Replaced search_with_quality with search_with_ef

## Decisions Made
- velesdb-core README exports (Filter, Condition, recall_at_k, precision_at_k, mrr, ndcg_at_k) verified correct against lib.rs -- no changes needed (N1, N2 confirmed accurate)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All 3 plans in Phase 18 documentation audit complete
- All code snippets in READMEs, guides, and docs verified against real API surface

---
## Self-Check: PASSED

All 5 modified files exist on disk. Both task commits (38b9ec99, 324acc4c) verified in git log.

---
*Phase: 18-documentation-code-audit*
*Completed: 2026-03-08*
