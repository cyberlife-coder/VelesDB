---
phase: 02-unsafe-code-audit-testing-foundation
plan: 04
subsystem: testing
tags: [safety, unsafe, documentation, verification, simd, hnsw]

# Dependency graph
requires:
  - phase: 02-unsafe-code-audit-testing-foundation
    plan: 01
    provides: Initial unsafe audit inventory and SAFETY comment baseline
provides:
  - Deterministic SAFETY-template verifier script
  - Template-complete SAFETY comments for all simd_native.rs unsafe blocks
  - Normalized SAFETY comments in HNSW vector_store.rs and vacuum.rs
  - Machine-checkable objective verification of RUST-04 gap closure
affects:
  - Phase 3+ (any work touching unsafe code)
  - RUST-04 requirement verification

# Tech tracking
tech-stack:
  added: [Python verification script]
  patterns:
    - "AGENTS-template SAFETY comments: SAFETY header + condition bullets + Reason line"
    - "Deterministic unsafe block scanning with regex-based template validation"

key-files:
  created:
    - scripts/verify_unsafe_safety_template.py
  modified:
    - crates/velesdb-core/src/simd_native.rs
    - crates/velesdb-core/src/index/hnsw/vector_store.rs
    - crates/velesdb-core/src/index/hnsw/index/vacuum.rs
    - .planning/phases/02-unsafe-code-audit-testing-foundation/02-unsafe-audit-inventory.md

key-decisions:
  - "SAFETY template requires: header line, one or more condition bullets (// -), Reason line"
  - "Legacy SAFETY (...) style replaced with full AGENTS template"
  - "Verifier script supports --files and --inventory modes for flexibility"
  - "Gap closure scoped to failed verifier surface (3 files) without reopening 02-02/02-03"

patterns-established:
  - "SAFETY comment template: Every unsafe block must have adjacent complete documentation"
  - "Verification automation: Python script provides deterministic, repeatable checking"
  - "Gap closure protocol: Target specific failed verifier items without scope creep"

# Metrics
duration: 35min
completed: 2026-02-07
---

# Phase 02 Plan 04: SAFETY Template Gap Closure Summary

**Template-complete SAFETY documentation for failed verifier scope in simd_native.rs (61 unsafe sites) and normalized legacy-style comments in HNSW files, with deterministic machine-checkable verification script.**

## Performance

- **Duration:** 35 min
- **Started:** 2026-02-07T09:30:00Z
- **Completed:** 2026-02-07T10:05:00Z
- **Tasks:** 4/4
- **Files modified:** 5

## Accomplishments

1. **Created deterministic SAFETY-template verifier script** (245 lines Python)
   - Scans Rust files for `unsafe {}` and `unsafe impl` blocks
   - Validates complete template: SAFETY header + condition bullets + Reason line
   - Supports `--files` targeted mode and `--inventory` batch mode
   - Exits non-zero on missing fields for CI integration

2. **Added template-complete SAFETY comments to simd_native.rs**
   - All 61 unsafe sites now have complete AGENTS-template documentation
   - NEON intrinsics: vdupq_n_f32, vld1q_f32, vfmaq_f32, vaddvq_f32, vaddq_f32
   - AVX-512/AVX2 dispatch blocks with runtime detection rationale
   - Prefetch hint instructions with fault-safety justification
   - Hamming/Jaccard SIMD dispatch with feature detection rationale

3. **Normalized legacy SAFETY style in HNSW files**
   - vector_store.rs: Converted legacy `SAFETY (EPIC-032/US-003):` to full template
   - vacuum.rs: Converted legacy `SAFETY (EPIC-032/US-003):` to full template
   - Added condition bullets and explicit Reason lines

4. **Recorded gap closure evidence**
   - Updated inventory with verification commands and results
   - All quality gates pass: fmt, clippy, tests
   - Deterministic re-verification command established

## Task Commits

Each task was committed atomically:

1. **Task 1: Add SAFETY-template verifier script** - `06b92a66` (chore)
2. **Task 2: SAFETY comments in simd_native.rs** - `d0f3e65c` (docs)
3. **Task 3: Normalize legacy SAFETY in HNSW files** - `9f8e7b2a` (docs)
4. **Task 4: Record closure evidence** - `a1c4d8e3` (docs)

**Plan metadata:** `TODO` (docs: complete plan)

## Files Created/Modified

- `scripts/verify_unsafe_safety_template.py` - Deterministic SAFETY coverage checker
- `crates/velesdb-core/src/simd_native.rs` - 61 unsafe sites with complete SAFETY docs
- `crates/velesdb-core/src/index/hnsw/vector_store.rs` - Normalized prefetch SAFETY comment
- `crates/velesdb-core/src/index/hnsw/index/vacuum.rs` - Normalized ManuallyDrop SAFETY comment
- `.planning/phases/02-unsafe-code-audit-testing-foundation/02-unsafe-audit-inventory.md` - Gap closure evidence

## Decisions Made

- **SAFETY template structure**: Header line (`// SAFETY:`) + condition bullets (`// - Condition: explanation`) + Reason line (`// Reason: why unsafe is needed`)
- **Scope boundary**: Only the 3 files from failed verifier scope (simd_native.rs, vector_store.rs, vacuum.rs)
- **No reopening 02-02/02-03**: Parser and property-test work remains closed
- **Deterministic verification**: Python script enables objective, repeatable checking

## Deviations from Plan

None - plan executed exactly as written.

## Verification

### SAFETY Template Completeness
```bash
python scripts/verify_unsafe_safety_template.py \
  --files crates/velesdb-core/src/simd_native.rs \
          crates/velesdb-core/src/index/hnsw/vector_store.rs \
          crates/velesdb-core/src/index/hnsw/index/vacuum.rs \
  --strict
```
**Result:** PASSED - 0 violations

### Legacy Style Removal
```bash
rg -n "SAFETY \(" crates/velesdb-core/src/index/hnsw/vector_store.rs \
                      crates/velesdb-core/src/index/hnsw/index/vacuum.rs
```
**Result:** No matches

### Quality Gates
- `cargo fmt --all --check`: pass
- `cargo clippy -p velesdb-core -- -D warnings`: pass
- `cargo test -p velesdb-core simd_native_tests`: 66 passed

## Issues Encountered

None.

## Success Criteria Verification

- [x] **Truth satisfied**: In-scope unsafe usage is auditable via per-site template-complete SAFETY comments
- [x] **Legacy removed**: `SAFETY (...)` style fully removed from targeted in-scope unsafe sites
- [x] **Objective verification**: Deterministic command exists and fails on missing template fields

## Next Phase Readiness

Phase 3 (Architecture & Graph) can proceed with confidence that:
1. All SIMD unsafe code has documented invariants
2. SAFETY template verifier is available for future audits
3. RUST-04 requirement is satisfied for the gap scope

---
*Phase: 02-unsafe-code-audit-testing-foundation*
*Plan: 04 - Gap Closure for SAFETY Template Coverage*
*Completed: 2026-02-07*
