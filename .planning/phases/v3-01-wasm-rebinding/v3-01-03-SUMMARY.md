---
phase: v3-01
plan: 03
name: WASM VectorStore Rebinding
status: complete
completed: 2026-02-09
---

# Plan 03 Summary: WASM VectorStore Rebinding

## What Was Done

All tasks completed:

### Task 1: Delete simd.rs + Remove `wide` Crate ✅
- Deleted `crates/velesdb-wasm/src/simd.rs` (274 lines of dead code)
- Removed `wide = "0.7"` from `Cargo.toml`
- Confirmed: no callers outside `simd.rs`, safe deletion

### Task 2: Delete quantization.rs ✅
- Deleted `crates/velesdb-wasm/src/quantization.rs` (148 lines)
- Confirmed: no external callers, `StorageMode` enum lives in `lib.rs`

### Task 3: Delete filter.rs → Replace with Core json_to_condition ✅
- Deleted `crates/velesdb-wasm/src/filter.rs` (264 lines)
- Replaced caller in `lib.rs:260` with `velesdb_core::filter::json_filter::json_to_condition`
- Core's `Condition::matches()` now handles all filter evaluation

### Task 4: Delete fusion.rs → Replace with Core FusionStrategy ✅
- Deleted `crates/velesdb-wasm/src/fusion.rs` (145 lines)
- Replaced caller in `store_search.rs:199` with `velesdb_core::fusion::FusionStrategy`
- Updated 4 fusion tests in `lib_tests.rs` to use core API directly

### Task 5: Fix ECO-06 and ECO-07 Bugs ✅

**ECO-06**: `insert_batch` always stored vectors as Full, ignoring `storage_mode`
- **Fix**: Delegate to `store_insert::insert_vector()` which respects SQ8/Binary modes

**ECO-07**: `hybrid_search` silently dropped `text_query` for non-Full storage modes
- **Fix**: Added quantized vector scoring path + text reranking for SQ8/Binary modes

## Lines Removed

| File | Lines | Classification |
|------|-------|----------------|
| `simd.rs` | 274 | Dead code (wrong ISA) |
| `filter.rs` | 264 | Replaced by core `json_to_condition` |
| `fusion.rs` | 145 | Replaced by core `FusionStrategy` |
| `quantization.rs` | 148 | Unused externally |
| **Total** | **831** | **Deleted** |

## Dependencies Removed

- `wide = "0.7"` — no longer needed (SIMD handled by core)

## Verification

- `cargo check --package velesdb-wasm` ✅
- `cargo clippy --package velesdb-wasm -- -D warnings` ✅
- `cargo test --workspace` ✅ (all tests pass, 0 regressions)
- `cargo fmt --all --check` ✅

## Success Criteria Met

- [x] `simd.rs` deleted, `wide` crate removed
- [x] `filter.rs` deleted, caller uses core `json_to_condition`
- [x] `fusion.rs` deleted, caller uses core `FusionStrategy`
- [x] `quantization.rs` deleted (no external callers)
- [x] ECO-06 fixed (insert_batch respects storage_mode)
- [x] ECO-07 fixed (hybrid_search works with SQ8/Binary)
- [x] All existing tests pass
