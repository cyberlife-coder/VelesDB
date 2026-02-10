# Plan 04-01: HNSW & Search Performance — SUMMARY

**Status:** ✅ Complete  
**Date:** 2026-02-10  
**Findings addressed:** D-02 (HNSW lock contention), D-04 (hardcoded overfetch)

---

## What was done

### Task 1: Benchmark baseline saved
- **Ran** `cargo bench -p velesdb-core --bench hnsw_benchmark -- --save-baseline before-phase4`
- **Saved** 18 benchmark groups as `before-phase4` baseline in `target/criterion/`
- Key measurements: insert sequential 764ms/1000, search 10k top10 ~70µs

### Task 2: HNSW single read lock per search [D-02]
- **Refactored** `search_layer_single`: acquire `vectors.read()` + `layers.read()` once for entire greedy descent (was: `layers.read()` per loop iteration + `get_vector()` per neighbor)
- **Refactored** `search_layer`: acquire `layers.read()` once before the while loop (was: `self.layers.read()` per candidate pop)
- **Lock ordering maintained**: vectors (rank 10) → layers (rank 20), released in reverse order
- **Added** proper `record_lock_acquire`/`record_lock_release` calls for both locks in both functions

### Task 3: Adaptive over-fetch factor [D-04]
- **Added** `get_overfetch()` to `WithClause`: returns overfetch multiplier, clamped to 1-100
- **Replaced** hardcoded `10 * count` with `overfetch_base * count` in `execute_query()` (2 match arms)
- **Threaded** `overfetch_base` parameter through `execute_union_query()`
- **Default:** 10 (backward compatible)
- **Usage:** `SELECT ... LIMIT 10 WITH (overfetch = 20)`
- **5 tests added:** default (None), custom value, clamped min, clamped max, VelesQL parsing

---

## Files modified

| File | Change |
|------|--------|
| `crates/velesdb-core/src/index/hnsw/native/graph/search.rs` | D-02: Single lock acquisition per search in both `search_layer_single` and `search_layer` |
| `crates/velesdb-core/src/velesql/ast/with_clause.rs` | D-04: Added `get_overfetch()` getter |
| `crates/velesdb-core/src/collection/search/query/mod.rs` | D-04: Extract overfetch from WITH clause, replace hardcoded 10 |
| `crates/velesdb-core/src/collection/search/query/union_query.rs` | D-04: Accept `overfetch_base` parameter |
| `crates/velesdb-core/src/velesql/with_options_tests.rs` | D-04: 5 tests for overfetch getter and parsing |

---

## Verification

- ✅ `cargo fmt --all --check` — clean
- ✅ `cargo clippy --workspace --features persistence,gpu,update-check --exclude velesdb-python -- -D warnings` — clean
- ✅ `cargo deny check` — advisories ok, bans ok, licenses ok, sources ok
- ✅ `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python` — all tests pass
- ✅ HNSW graph tests pass: `test_insert_and_search`, `test_concurrent_insertions`, `test_concurrent_insert_and_search`
- ✅ Overfetch tests pass: 5/5 (default, custom, clamped min/max, parsing)
- ✅ Benchmark baseline saved: 18 groups in `before-phase4`
