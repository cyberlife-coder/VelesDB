---
phase: 02-pq-core-engine
plan: 02
subsystem: quantization
tags: [pq, adc, simd, avx2, neon, lut, oversampling, rescore]

# Dependency graph
requires:
  - phase: 02-pq-core-engine/01
    provides: "ProductQuantizer with codebook, quantize/reconstruct, persistence"
provides:
  - "precompute_lut method on ProductQuantizer (flat m*k LUT with OPQ rotation)"
  - "adc_distances_batch SIMD module (AVX2 gather, NEON, scalar dispatch)"
  - "Configurable pq_rescore_oversampling field on CollectionConfig (default 4x)"
affects: [02-pq-core-engine/03, 02-pq-core-engine/04]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "ADC LUT precomputation separated from per-candidate evaluation"
    - "SIMD dispatch via simd_level() for ADC gather kernels"
    - "serde default function for backward-compatible config evolution"

key-files:
  created:
    - crates/velesdb-core/src/simd_native/adc.rs
  modified:
    - crates/velesdb-core/src/quantization/pq.rs
    - crates/velesdb-core/src/simd_native/mod.rs
    - crates/velesdb-core/src/collection/types.rs
    - crates/velesdb-core/src/collection/search/vector.rs
    - crates/velesdb-core/src/collection/core/lifecycle.rs

key-decisions:
  - "AVX2 gather uses _mm256_i32gather_ps with scale=4, processing 8 subspaces per iteration"
  - "LUT is flat Vec<f32> indexed as lut[subspace * k + centroid_id] for cache-friendly access"
  - "Default oversampling lowered from hardcoded 8x to configurable 4x (sufficient with ADC)"
  - "None disables rescore entirely as expert-only escape hatch"

patterns-established:
  - "ADC SIMD dispatch: match simd_level() for AVX2/NEON/scalar with SAFETY comments"
  - "Config evolution: serde default functions for backward-compatible field addition"

requirements-completed: [PQ-02, PQ-04]

# Metrics
duration: 26min
completed: 2026-03-06
---

# Phase 2 Plan 02: ADC SIMD + Configurable Rescore Summary

**SIMD-accelerated ADC distance with AVX2 gather for PQ search, plus configurable 4x rescore oversampling replacing hardcoded 8x**

## Performance

- **Duration:** 26 min
- **Started:** 2026-03-06T10:34:23Z
- **Completed:** 2026-03-06T11:00:43Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Built precompute_lut on ProductQuantizer: flat m*k LUT with rotation support, computed once per query
- Created simd_native/adc.rs with AVX2 gather (_mm256_i32gather_ps), NEON 4-wide, and scalar fallback
- Made rescore oversampling configurable via pq_rescore_oversampling (default Some(4), down from hardcoded 8x)
- Ensured backward compatibility: existing serialized configs without the field get Some(4)

## Task Commits

Each task was committed atomically:

1. **Task 1: LUT precomputation + ADC SIMD module** - `3b5e734f` (feat)
2. **Task 2: Configurable rescore oversampling with 4x default** - `1dcd32fe` (feat)

## Files Created/Modified
- `crates/velesdb-core/src/simd_native/adc.rs` - New ADC SIMD module with AVX2/NEON/scalar paths
- `crates/velesdb-core/src/quantization/pq.rs` - Added precompute_lut and apply_rotation methods
- `crates/velesdb-core/src/simd_native/mod.rs` - Registered adc module, added public re-export
- `crates/velesdb-core/src/collection/types.rs` - Added pq_rescore_oversampling field to CollectionConfig
- `crates/velesdb-core/src/collection/search/vector.rs` - Configurable oversampling in search pipeline
- `crates/velesdb-core/src/collection/core/lifecycle.rs` - Set default oversampling in collection constructors

## Decisions Made
- Used `_mm256_i32gather_ps` with scale=4 for AVX2 ADC (8 subspaces per iteration)
- LUT indexed as flat `lut[subspace * k + centroid_id]` for cache-friendly sequential access
- Default oversampling lowered from 8x to 4x (sufficient with proper ADC, reduces wasted work)
- None value for oversampling disables rescore entirely (expert-only, documented risk)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Re-exported RaBitQ types to fix dead_code warnings**
- **Found during:** Task 1 (commit hook)
- **Issue:** Pre-existing dead_code warnings in rabitq.rs blocked commit via -D warnings
- **Fix:** Added `pub use rabitq::{RaBitQIndex, RaBitQVector}` to quantization/mod.rs
- **Files modified:** crates/velesdb-core/src/quantization/mod.rs
- **Verification:** Clippy passes, commit succeeds
- **Committed in:** 3b5e734f (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Minor scope addition to unblock commit. No scope creep.

## Issues Encountered
- AVX2 `_mm256_i32gather_ps` API uses const generic for scale parameter (not runtime argument) -- fixed immediately
- Pre-commit hook's full test suite run adds ~2 min per commit attempt

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- ADC pipeline ready for integration with HNSW PQ search path
- precompute_lut + adc_distances_batch form the hot-path pair for Plan 03 (PQ-HNSW integration)
- Configurable rescore oversampling ready for VelesQL TRAIN command exposure in Plan 04

---
*Phase: 02-pq-core-engine*
*Completed: 2026-03-06*
