---
phase: 03-architecture-extraction-graph-safety
plan: 04
subsystem: testing
tags: [concurrency, hnsw, storage, guard, epoch, loom, deadlock, soft-delete]

# Dependency graph
requires:
  - phase: 03-architecture-extraction-graph-safety
    provides: Lock-order runtime checker, safety counters, graph submodule extraction (Plan 03-03)
provides:
  - Concurrency family 1: parallel insert/search/delete tests with deterministic invariant assertions
  - Concurrency family 2: resize/snapshot consistency tests for VectorSliceGuard epoch behavior
  - Loom-backed epoch guard invalidation scenarios for deterministic interleaving verification
affects: [phase-4-complexity-errors, phase-5-cleanup-performance]

# Tech tracking
tech-stack:
  added: []
  patterns: [deterministic concurrency post-conditions, epoch-based stale-guard detection, soft-delete exclusion testing]

key-files:
  created: []
  modified:
    - crates/velesdb-core/src/index/hnsw/native/tests.rs
    - crates/velesdb-core/src/index/hnsw/native/graph_tests.rs
    - crates/velesdb-core/src/storage/tests.rs
    - crates/velesdb-core/src/storage/loom_tests.rs

key-decisions:
  - "Test soft-delete at NativeHnswIndex level (not NativeHnsw graph) since remove is a mapping-layer operation"
  - "Assert graph-size invariants instead of logical-size for NativeHnswIndex::len() (which returns HNSW graph count, not mappings count)"
  - "Loom tests verify epoch semantics under deterministic interleavings; standard tests verify full storage integration"

patterns-established:
  - "Concurrency tests must include deterministic post-conditions (not just no-panic)"
  - "Safety counter assertions (zero invariant violations) included in concurrent test epilogues"
  - "Soft-delete exclusion verified via post-operation search result filtering"

# Metrics
duration: 10min
completed: 2026-02-07
---

# Phase 3 Plan 4: Concurrency Verification Coverage Summary

**Two required concurrency test families (insert/search/delete + resize/snapshot) with deterministic invariant assertions exercising lock-order checker and safety counters from Plan 03-03**

## Performance

- **Duration:** ~10 min
- **Started:** 2026-02-07T15:52:42Z
- **Completed:** 2026-02-07T16:02:48Z
- **Tasks:** 3
- **Files modified:** 4

## Accomplishments
- Implemented Concurrency Family 1: 6 new parallel insert/search/delete tests across graph-level (`graph_tests.rs`) and index-level (`tests.rs`), including soft-delete exclusion verification
- Implemented Concurrency Family 2: 5 new resize/snapshot consistency tests in `storage/tests.rs` plus 3 loom-backed epoch guard scenarios in `loom_tests.rs`
- All tests include deterministic post-conditions: count assertions, distance sorting, node ID validity, epoch staleness detection, and safety counter zero-violation checks
- Verified zero lock-order violations across all concurrent test scenarios

## Task Commits

Each task was committed atomically:

1. **Task 1: Add parallel insert/search/delete concurrency suite** - `7b7a62f1` (test)
2. **Task 2: Add resize/snapshot consistency tests** - `9d79519d` (test)
3. **Task 3: Safety regression and quality checks** - No code changes (verification-only task: all checks pass)

## Files Created/Modified
- `crates/velesdb-core/src/index/hnsw/native/tests.rs` — 6 new concurrency tests: deterministic count, correctness assertions, multi-entry search, insert+delete+search at index level, delete exclusion verification
- `crates/velesdb-core/src/index/hnsw/native/graph_tests.rs` — 1 new graph-level parallel insert+search integrity test with safety counter check
- `crates/velesdb-core/src/storage/tests.rs` — 5 new tests: guard invalidation after resize, epoch increment tracking, concurrent snapshot reads, epoch mismatch detection, interleaved store+read
- `crates/velesdb-core/src/storage/loom_tests.rs` — 3 new loom-backed tests: guard sees epoch bump, stale after multiple resizes, valid until next resize

## Decisions Made
- Test soft-delete at `NativeHnswIndex` level (not `NativeHnsw` graph) since `remove()` only operates on the mappings layer — graph nodes are retained as tombstones
- `NativeHnswIndex::len()` returns graph size (not logical count after deletes), so assertions use graph size + search exclusion instead of mapping count
- Loom tests target epoch semantics only (not full MmapStorage) since loom doesn't support file I/O — standard integration tests cover the full stack

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed incorrect count assertion in delete-aware concurrency test**
- **Found during:** Task 1
- **Issue:** Initial assertion expected `NativeHnswIndex::len()` to reflect soft-deletes, but len() returns the inner HNSW graph size which doesn't decrease on soft-delete
- **Fix:** Changed assertions to verify graph size equals total inserts, and verify soft-delete exclusion via search result filtering instead
- **Files modified:** `crates/velesdb-core/src/index/hnsw/native/tests.rs`
- **Verification:** Both delete-aware tests pass with correct assertions
- **Committed in:** `7b7a62f1`

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Fix was necessary for correct test semantics. No scope creep.

## Issues Encountered
None — all quality gates passed on first attempt after the initial assertion fix.

## User Setup Required
None — no external service configuration required.

## Next Phase Readiness
- Phase 3 is complete: all 4 plans (extraction + safety + concurrency) delivered
- Both locked concurrency test families are implemented and passing
- Lock-order runtime checker from Plan 03-03 is exercised by concurrent tests
- Safety counters verified as zero-violation across all concurrent scenarios
- Ready for Phase 4: Complexity Reduction & Error Handling

---
*Phase: 03-architecture-extraction-graph-safety*
*Completed: 2026-02-07*

## Self-Check: PASSED
