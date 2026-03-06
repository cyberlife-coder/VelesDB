---
phase: 04-sparse-vector-engine
plan: 02
subsystem: database
tags: [sparse-search, maxscore, daat, inverted-index, criterion, benchmark]

# Dependency graph
requires:
  - phase: 04-sparse-vector-engine/01
    provides: SparseInvertedIndex, SparseVector, PostingEntry, ScoredDoc types and inverted index
provides:
  - sparse_search() public API with MaxScore DAAT and linear scan fallback
  - Criterion benchmark suite for sparse insert and search throughput
  - brute_force_search() test utility for correctness validation
affects: [04-sparse-vector-engine/03, 05-sparse-integration]

# Tech tracking
tech-stack:
  added: []
  patterns: [MaxScore DAAT with essential/non-essential term partitioning, coverage-based strategy selection, dense vs hashmap accumulator]

key-files:
  created:
    - crates/velesdb-core/src/index/sparse/search.rs
    - crates/velesdb-core/benches/sparse_benchmark.rs
  modified:
    - crates/velesdb-core/src/index/sparse/mod.rs
    - crates/velesdb-core/Cargo.toml
    - crates/velesdb-core/src/collection/types.rs

key-decisions:
  - "MaxScore uses sorted-by-contribution term ordering with prefix-sum upper bounds for early termination"
  - "Linear scan threshold at 30% coverage (total_postings > 0.3 * doc_count * query_nnz)"
  - "Dense array accumulator up to 10M doc IDs, FxHashMap fallback above"

patterns-established:
  - "MaxScore DAAT: sort terms by max_contribution ascending, prefix-sum upper bounds, essential/non-essential split"
  - "Strategy selection: coverage heuristic determines MaxScore vs linear scan at query time"

requirements-completed: [SPARSE-03]

# Metrics
duration: 22min
completed: 2026-03-06
---

# Phase 04 Plan 02: Sparse Search Summary

**MaxScore DAAT search with linear scan fallback over inverted index, validated against brute-force on 1K SPLADE corpus**

## Performance

- **Duration:** 22 min
- **Started:** 2026-03-06T17:47:10Z
- **Completed:** 2026-03-06T18:09:16Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- MaxScore DAAT search returning identical results to brute-force across 50 queries on 1K corpus
- Linear scan fallback auto-activating for high-coverage queries (>30% of collection)
- Criterion benchmark suite: 10K doc insert (~73ms seq), top-10 search (~846us), 16-thread concurrent workload
- All 9 search unit tests passing including brute-force comparison

## Task Commits

Each task was committed atomically:

1. **Task 1: MaxScore DAAT search + linear scan fallback** - `d30af820` (feat)
2. **Task 2: Sparse Criterion benchmark suite** - `6886428f` (feat)

## Files Created/Modified
- `crates/velesdb-core/src/index/sparse/search.rs` - MaxScore DAAT search, linear scan fallback, brute-force test utility (520 lines)
- `crates/velesdb-core/src/index/sparse/mod.rs` - Re-exports search module and sparse_search function
- `crates/velesdb-core/benches/sparse_benchmark.rs` - Criterion benchmarks for insert, search, and concurrent workloads
- `crates/velesdb-core/Cargo.toml` - Added sparse_benchmark bench entry
- `crates/velesdb-core/src/collection/types.rs` - Added dead_code allow on sparse_index accessor

## Decisions Made
- MaxScore uses sorted-by-contribution term ordering with prefix-sum upper bounds for early termination
- Linear scan threshold at 30% coverage (consistent with academic DAAT literature)
- Dense array accumulator for doc IDs up to 10M, FxHashMap fallback for larger spaces to avoid excessive memory
- Module-level `#![allow(clippy::cast_precision_loss)]` since f32 casts are intentional throughout search scoring

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed dead_code warning on sparse_index accessor**
- **Found during:** Task 1 (search implementation)
- **Issue:** `sparse_index()` accessor on Collection added in plan 04-01 was unused, causing clippy -D warnings to fail pre-commit hook
- **Fix:** Added `#[allow(dead_code)]` with doc comment noting it is used in the sparse integration phase
- **Files modified:** crates/velesdb-core/src/collection/types.rs
- **Verification:** cargo clippy passes with -D warnings
- **Committed in:** d30af820 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Minor fix to unblock pre-commit hook. No scope creep.

## Issues Encountered
- Pre-commit hook runs full workspace clippy and tests, adding ~3 min per commit. Pre-existing uncommitted changes in lifecycle.rs, persistence.rs, database.rs from prior sessions required careful selective staging.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- sparse_search() API ready for VelesQL integration in phase 05 (sparse integration)
- Benchmark baseline established: ~846us/query top-10 on 10K SPLADE corpus
- 16-thread concurrent workload demonstrates no deadlocks under segment-level isolation

---
*Phase: 04-sparse-vector-engine*
*Completed: 2026-03-06*
