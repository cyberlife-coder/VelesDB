# Plan 02-03: Fusion Unification & ParseError — SUMMARY

**Status:** ✅ Complete  
**Date:** 2026-02-09  
**Findings addressed:** D-09, C-04/B-03 follow-up

---

## What was done

### Task 1: Delete old FusionStrategy enum
- **Deleted** the broken `FusionStrategy` enum from `score_fusion/mod.rs` (6 variants + `combine()` impl with fake RRF formula `1/(k+(1-score)*100)`)
- **Imported** `crate::fusion::FusionStrategy` as the single source of truth
- **Rewired** `ScoreBreakdown::compute_final()` to use `FusionStrategy::fuse()` by converting score components into pseudo-query results
- **Added** `as_str()` method to `crate::fusion::FusionStrategy` for explanation API compatibility
- **Updated** `explanation.rs` — resolves via `super::FusionStrategy` (re-exported)

### Task 2: Fix fusion parameter parsing
- **Fixed** `conditions.rs:224`: `unwrap_or(0.0)` → `map_err(|_| ParseError::invalid_value(...))?`
- **Fixed** `match_parser.rs:197,202`: both `unwrap_or(0)` and `unwrap_or(0.0)` → `map_err(|_| ParseError::invalid_value(...))?`
- **Added** `ParseErrorKind::InvalidValue` (E007) with constructor `ParseError::invalid_value()`
- **Changed** `parse_fusion_clause` return type: `FusionConfig` → `Result<FusionConfig, ParseError>`

### Task 3: Regression tests
- **Added** `test_rrf_regression_rank_based_not_score_based` — proves RRF uses `1/(k+rank+1)`, not the broken score-based formula
- **Added** `test_weighted_differs_from_average` — proves non-uniform weights produce different results
- **Added** `test_fusion_strategy_as_str_all_variants` — covers new `as_str()` method
- **Added** `test_parse_error_invalid_value` — covers new E007 error variant
- **Updated** `score_fusion_tests.rs`: removed Minimum/Product tests (variants deleted), added RRF/Weighted tests with correct variant syntax
- **Updated** `test_parse_error_kind_codes` to include E007

---

## Files modified

| File | Change |
|------|--------|
| `crates/velesdb-core/src/collection/search/query/score_fusion/mod.rs` | Deleted broken enum, imported correct FusionStrategy, rewired compute_final |
| `crates/velesdb-core/src/fusion/strategy.rs` | Added `as_str()` method |
| `crates/velesdb-core/src/velesql/error.rs` | Added InvalidValue variant (E007) + constructor + tests |
| `crates/velesdb-core/src/velesql/parser/conditions.rs` | Fixed unwrap_or(0.0), changed return type to Result |
| `crates/velesdb-core/src/velesql/parser/match_parser.rs` | Fixed unwrap_or(0) and unwrap_or(0.0) |
| `crates/velesdb-core/src/collection/search/query/score_fusion_tests.rs` | Updated for new FusionStrategy variants |
| `crates/velesdb-core/src/fusion/strategy_tests.rs` | Added 3 regression tests |

---

## Verification

- ✅ `cargo fmt --all --check` — clean
- ✅ `cargo clippy --package velesdb-core -- -D warnings` — clean
- ✅ `cargo deny check` — advisories ok, bans ok, licenses ok, sources ok
- ✅ `cargo test --package velesdb-core` — all tests pass
- ✅ Zero matches for `1.0 - s.*100` in score_fusion/mod.rs
- ✅ Zero matches for `unwrap_or(0.0)` in conditions.rs and match_parser.rs
- ✅ Only ONE `FusionStrategy` type in crate: `crate::fusion::FusionStrategy`
