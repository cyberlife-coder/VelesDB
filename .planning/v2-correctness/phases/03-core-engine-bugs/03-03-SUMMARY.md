# Plan 03-03 Summary: Quantization & DualPrecision Fixes

**Status:** ✅ Complete  
**Commit:** `df724ea6`  
**Findings:** B-04, B-06, D-08  

## Changes

### B-04: DualPrecision default search uses int8
- **File:** `index/hnsw/native/dual_precision.rs`
- **Fix:** Replaced `search()` method body (which used `search_dual_precision` with f32 traversal) with delegation to `search_with_config()` using `DualPrecisionConfig::default()` (which has `use_int8_traversal: true`)
- **Impact:** Default `search()` now uses int8 graph traversal when quantizer is trained, providing the 4x bandwidth reduction promised by VSAG paper

### B-06: cosine_similarity_quantized — no full dequantization for norm
- **File:** `quantization/scalar.rs`
- **Fix:** Both `cosine_similarity_quantized` and `cosine_similarity_quantized_simd` now compute the quantized vector norm directly from int8 data using algebraic expansion: `norm² = scale²·Σq² + 2·scale·offset·Σq + n·offset²`
- **Impact:** Eliminates `Vec<f32>` allocation per call (was `to_f32()` which allocated dimension×4 bytes). Also added `.clamp(-1.0, 1.0)` on output for numerical stability.

### D-08: Rename QuantizedVector → QuantizedVectorInt8
- **Files:** `index/hnsw/native/quantization.rs`, `mod.rs`, `dual_precision.rs`, `quantization_tests.rs`
- **Fix:** Renamed `QuantizedVector` to `QuantizedVectorInt8` and `QuantizedVectorStore` to `QuantizedVectorInt8Store` in the HNSW native module
- **Impact:** Disambiguates from `quantization::scalar::QuantizedVector` (SQ8, per-vector min/max) vs HNSW native (per-dimension min/max via ScalarQuantizer)

## Deviations
- None
