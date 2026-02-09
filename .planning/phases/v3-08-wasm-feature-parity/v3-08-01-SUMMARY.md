# Plan 01 Summary: Extract ColumnStore from Persistence Gate

## Status: ✅ COMPLETED

## Changes

### `crates/velesdb-core/src/column_store/mod.rs`
- Gated `from_collection` and `from_collection_tests` behind `#[cfg(feature = "persistence")]`
- Rest of `column_store` module now unconditionally available

### `crates/velesdb-core/src/lib.rs`
- Removed `#[cfg(feature = "persistence")]` from `pub mod column_store` and its re-exports
- `column_store_tests` now run under `#[cfg(test)]` unconditionally

## Verification
- `cargo check --package velesdb-core --no-default-features` ✅
- `cargo check --package velesdb-wasm` ✅
- `cargo test --workspace` ✅
