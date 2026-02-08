# Plan 03-01 Summary: Input Validation & VelesQL Fixes

**Status:** ✅ Complete  
**Commit:** `2d96333e`  
**Findings:** B-01, B-02  

## Changes

### B-01: Block NaN/Infinity vectors in extraction
- **File:** `collection/search/query/extraction.rs`
- **Fix:** Changed 3 `else` branches (in `extract_vector_search`, `extract_all_similarity_conditions`, `resolve_vector`) to return `None` instead of casting non-finite `f64` to `f32`
- **Impact:** NaN/Infinity vectors now produce clear errors instead of silently corrupting similarity calculations

### B-02: ORDER BY property paths → UnsupportedFeature error
- **File:** `collection/search/query/match_exec/similarity.rs`
- **Fix:** Changed `order_match_results` return type from `()` to `Result<()>`, catch-all `_` now returns `Error::UnsupportedFeature` (VELES-027)
- **File:** `error.rs` — Added `UnsupportedFeature(String)` variant with code VELES-027

### Regression Tests (9 tests)
- **File:** `collection/search/query/extraction_tests.rs`
- NaN, Infinity, NEG_INFINITY vector rejection in vector_search, similarity_condition, resolve_vector
- Valid vector passes extraction
- ORDER BY property path returns error
- ORDER BY similarity/depth succeeds

## Deviations
- None
