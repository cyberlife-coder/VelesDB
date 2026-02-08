---
phase: 7
plan: 1
completed: 2026-02-07
duration: ~25min
---

# Phase 7 Plan 1: f64 Scalar Reference with Higham Error Bound — Summary

## One-liner

Replaced flaky f32-vs-f32 SIMD property tests with f64 ground-truth references and Higham's proven forward error bound, achieving 10/10 stability.

## What Was Built

The SIMD property tests previously compared f32 SIMD results against f32 scalar references using fixed tolerance constants. Both implementations accumulate rounding errors of the same magnitude but in different directions due to operation reordering (FMA, multi-accumulator, SIMD reduction). When both errors diverged, the delta exceeded the fixed tolerance, causing flaky failures.

The fix replaces all scalar references with f64 accumulation (52-bit mantissa vs f32's 23-bit), making the reference essentially exact for our vector sizes. Each proptest now measures the f32 SIMD result against this proper ground truth using Higham's mathematically proven forward error bound: `|error| ≤ γ(N) × condition_number` where `γ(N) = N × u / (1 - N × u)`.

Operation-specific multipliers account for extra per-term rounding: 3× for squared_l2 (subtraction + squaring), 3× for cosine (dot + 2 norms), and a proper ratio error propagation formula for Jaccard (`(ΔI + |I/U| × ΔU) / |U| + u × |I/U|`).

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | f64 ground-truth reference functions | 64fe7739 | simd_property_tests.rs |
| 2 | Higham error bound helper | 64fe7739 | simd_property_tests.rs |
| 3 | Rewrite proptests with f64 + Higham | 64fe7739 | simd_property_tests.rs |
| 4 | Update sanity test | 64fe7739 | simd_property_tests.rs |
| 5 | 10/10 stability verification | 64fe7739 | (verification only) |

## Key Files

**Modified:**
- `crates/velesdb-core/tests/simd_property_tests.rs` — Complete rewrite: f64 references, Higham bound, JaccardRef struct, ratio error propagation

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| 3× multiplier for squared_l2 | Each (a-b)² term has 3 rounding sources: subtraction error squared (2u) + multiplication (u) |
| 3× multiplier for cosine | dot computation + 2 norm computations, each with γ(N) error |
| Dedicated `jaccard_error_bound()` with ratio propagation | Simple multipliers insufficient when `\|result\| >> 1` (negative-value inputs); division amplifies numerator/denominator errors proportionally to `\|I/U\|` |
| Removed unused f32 scalars | `scalar_squared_l2`, `scalar_cosine`, `scalar_jaccard` no longer referenced after f64 migration; kept `scalar_dot` (sanity cross-check) and `scalar_hamming` (exact integer) |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Jaccard condition number was wrong**
- Found during: Task 5 (stability runs 2-10 failed)
- Issue: Condition number used `|Σmin| + |Σmax|` (final sums) instead of `Σ|min| + Σ|max|` (absolute individual terms), missing catastrophic cancellation
- Fix: Changed to sum of absolute individual terms
- Files: simd_property_tests.rs
- Commit: 64fe7739

**2. [Rule 1 - Bug] Jaccard ratio amplification not bounded**
- Found during: Task 5 (still failing after condition fix)
- Issue: When `|result| >> 1`, division amplifies rounding errors by `|I/U|`; a simple `3 × γ(N) × condition` multiplier was insufficient
- Fix: Added `JaccardRef` struct + `jaccard_error_bound()` with proper ratio error propagation formula
- Files: simd_property_tests.rs
- Commit: 64fe7739

**3. [Rule 1 - Bug] squared_l2 multiplier too small**
- Found during: Task 5 (initial run)
- Issue: 2× multiplier insufficient for squared_l2 — each term has 3 rounding sources, not 2
- Fix: Changed from 2× to 3× multiplier with documented rationale
- Files: simd_property_tests.rs
- Commit: 64fe7739

## Verification Results

```
cargo test --package velesdb-core --test simd_property_tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

10/10 consecutive runs: ALL PASS

cargo clippy --package velesdb-core -- -D warnings
0 warnings (clean)

cargo test --package velesdb-core simd_native
test result: ok. 106 passed; 0 failed; 3 ignored

Pre-commit hook: ALL PASS (fmt, clippy, workspace tests, secrets check)
```

## Next Phase Readiness

- **Plan 07-02** (Wire DistanceEngine into HNSW hot loop) is unblocked — tolerance hardening complete
- Phase 6 (DOCS-03, DOCS-04, PERF-02, PERF-03) still pending

---
*Completed: 2026-02-07T23:58+01:00*
