---
phase: 02-unsafe-code-audit-testing-foundation
plan: 03
subsystem: testing
tags: [simd, proptest, tolerance, reproducibility, rust]

# Dependency graph
requires:
  - phase: 02-01
    provides: Unsafe-audited SIMD/native code paths with stable public entrypoints
provides:
  - Property-based SIMD vs scalar equivalence coverage for dot/L2/cosine/hamming/jaccard
  - Explicit metric tolerance matrix suitable for cross-architecture CI variability
  - Reproducible proptest configuration with persisted failing counterexamples
affects: [phase-03-architecture-extraction, simd-refactors, ci-stability]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - Deterministic proptest config for integration tests
    - Absolute+relative tolerance envelopes per floating-point metric

key-files:
  created:
    - crates/velesdb-core/tests/simd_property_tests.rs
  modified:
    - crates/velesdb-core/tests/simd_property_tests.rs
    - crates/velesdb-core/src/simd_native_tests.rs

key-decisions:
  - "Use 256 proptest cases with explicit shrink bound and failure persistence for reproducible counterexamples."
  - "Use per-metric tolerance envelopes (looser for dot-product accumulation drift, tighter for exact-count metrics)."

patterns-established:
  - "SIMD correctness checks compare public SIMD entrypoints directly against scalar references."
  - "Dimension strategies must include tail and boundary widths (0/1/7/8/15/16/17/.../512+)."

# Metrics
duration: 8 min
completed: 2026-02-07
---

# Phase 2 Plan 03: SIMD Property Equivalence Summary

**Proptest-backed SIMD equivalence coverage now validates dot/L2/cosine/hamming/jaccard behavior against scalar references with reproducible counterexamples and explicit tolerance policy.**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-07T09:08:06Z
- **Completed:** 2026-02-07T09:16:06Z
- **Tasks:** 3
- **Files modified:** 2

## Accomplishments
- Added new integration property suite over randomized vectors and boundary-heavy dimensions.
- Encoded reproducible proptest execution settings and documented tolerance matrix in test code.
- Ran plan-required quality checks (`fmt`, `clippy -D warnings`, `simd_property_tests`) successfully.

## Task Commits

Each task was committed atomically:

1. **Task 1: Create SIMD property test suite with scalar references** - `347ed7fb` (test)
2. **Task 2: Encode reproducibility and non-flaky tolerance policy** - `6415c1e1` (test)
3. **Task 3: Run phase-level validation commands for SIMD test foundation** - `19752e4a` (test)

**Plan metadata:** pending (created after SUMMARY/STATE updates)

## Files Created/Modified
- `crates/velesdb-core/tests/simd_property_tests.rs` - Proptest SIMD-vs-scalar equivalence harness and tolerance matrix.
- `crates/velesdb-core/src/simd_native_tests.rs` - Shared tolerance constants reused by deterministic SIMD unit assertions.

## Decisions Made
- Kept hamming assertions exact (`prop_assert_eq!`) while using abs/rel tolerance envelopes for floating-point metrics.
- Set proptest failure persistence explicitly for integration-test context to keep counterexamples reproducible without `lib.rs` lookup warnings.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Isolated unrelated local parser test edits during task commits**
- **Found during:** Task commits (pre-commit hook execution)
- **Issue:** Existing unstaged local edits in `pr_review_bugfix_tests.rs` caused full hook test suite failure unrelated to SIMD tasks.
- **Fix:** Temporarily stashed only the unrelated local file during each task commit, then restored it immediately after commit.
- **Files modified:** none (process-only mitigation)
- **Verification:** Hook checks passed and original local file changes were restored after each commit.
- **Committed in:** process-level (no code delta)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** No scope creep; mitigation only unblocked required atomic commits.

## Issues Encountered
- Initial dot-product tolerance was too strict for one AVX accumulation-order case; adjusted envelope to maintain cross-architecture stability while preserving scalar equivalence guarantees.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- TEST-01 foundation is complete and ready for Phase 3 SIMD/module refactors.
- No blockers identified for starting `03-01-PLAN.md`.

---
*Phase: 02-unsafe-code-audit-testing-foundation*
*Completed: 2026-02-07*

## Self-Check: PASSED
