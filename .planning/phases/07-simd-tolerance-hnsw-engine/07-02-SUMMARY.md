# Plan 07-02 Summary: Full Zero-Dispatch DistanceEngine for HNSW

## Outcome: ✅ Complete

**Requirement:** PERF-04 — Wire DistanceEngine into HNSW hot loop  
**Branch:** feature/CORE-phase5-plan01-dependency-cleanup  

---

## What Changed

### Task 1: Extend `simd_native::DistanceEngine` (5b9a05db)
- Added `hamming_fn` and `jaccard_fn` fields to `DistanceEngine` struct
- Added `hamming()` and `jaccard()` public methods
- Added `resolve_hamming()` and `resolve_jaccard()` for SIMD kernel selection
- Updated `DistanceEngine::new()` to populate all 5 metrics

### Task 2: Unit tests for extended DistanceEngine (d6def7d8)
- `test_engine_hamming_matches_native()` — verifies cached dispatch matches native across 10 dimensions
- `test_engine_jaccard_matches_native()` — same for Jaccard

### Task 3: Create `CachedSimdDistance` (c7555885)
- New struct wrapping `simd_native::DistanceEngine`
- Implements HNSW `DistanceEngine` trait for all 5 metrics
- `batch_distance()` includes prefetch optimization
- Re-exported from `hnsw::native::mod.rs`

### Task 4: CachedSimdDistance parity tests (64cb823c)
- 5 metric-specific tests: cosine 768d, euclidean 128d, dot_product 1536d, hamming 64d, jaccard 256d
- 1 batch consistency test: batch_distance matches per-element loop
- All tests verify **exact bit-for-bit equality** with `SimdDistance`

### Task 5: Wire into NativeHnswBackend (3b77cdcc)
- `NativeHnswInner` now uses `NativeHnsw<CachedSimdDistance>` instead of `SimdDistance`
- Added `dimension` parameter to `NativeHnswInner::new()` and `file_load()`
- Updated 4 call sites: `native_index.rs`, `constructors.rs`, `vacuum.rs`

### Task 6: Verification
- **2,432 tests pass** (velesdb-core package)
- **313 HNSW tests pass** — zero regressions
- **6 SIMD property tests pass**
- **Clippy clean** (`-D warnings`)

---

## Files Modified

| File | Change |
|------|--------|
| `simd_native/dispatch.rs` | +2 fields, +2 methods, +2 resolvers |
| `simd_native/distance_engine_tests.rs` | +2 tests |
| `index/hnsw/native/distance.rs` | +CachedSimdDistance struct + trait impl |
| `index/hnsw/native/mod.rs` | +re-export CachedSimdDistance |
| `index/hnsw/native/distance_tests.rs` | +6 parity tests |
| `index/hnsw/native_inner.rs` | SimdDistance → CachedSimdDistance |
| `index/hnsw/native_index.rs` | Pass dimension to NativeHnswInner |
| `index/hnsw/index/constructors.rs` | Pass dimension to HnswInner |
| `index/hnsw/index/vacuum.rs` | Pass dimension to HnswInner |

---

## Architecture Impact

**Before (3 levels of dispatch per distance call):**
```
match metric → match simd_level() → match dimension >= threshold
```

**After (zero per-call dispatch):**
```
CachedSimdDistance.distance() → match metric → fn_pointer(a, b)
```

The `match metric` is a single perfectly-predicted branch (same metric for entire index lifetime). The `fn_pointer` call is a single indirect call with no further branching.

---

## Commits (5 atomic)

1. `5b9a05db` — feat(07-02): extend DistanceEngine with hamming_fn and jaccard_fn
2. `d6def7d8` — test(07-02): add hamming/jaccard tests for DistanceEngine
3. `c7555885` — feat(07-02): create CachedSimdDistance with cached fn pointer dispatch
4. `64cb823c` — test(07-02): add 6 CachedSimdDistance parity tests
5. `3b77cdcc` — feat(07-02): wire CachedSimdDistance into NativeHnswBackend
