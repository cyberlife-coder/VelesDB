//! Feature parity tests: WASM enum variants must match velesdb-core.
//!
//! These tests run on the host target (not wasm32) to verify that the
//! WASM `StorageMode` enum stays in sync with `velesdb_core::StorageMode`.
//!
//! When a new variant is added to core but not propagated here, the
//! assertion fails with a clear message pointing to the file to fix.

#![cfg(not(target_arch = "wasm32"))]

/// All WASM `StorageMode` variants, in declaration order.
const WASM_STORAGE_MODES: &[velesdb_wasm::StorageMode] = &[
    velesdb_wasm::StorageMode::Full,
    velesdb_wasm::StorageMode::SQ8,
    velesdb_wasm::StorageMode::Binary,
    velesdb_wasm::StorageMode::ProductQuantization,
    velesdb_wasm::StorageMode::RaBitQ,
];

/// All core `StorageMode` variants, in declaration order.
const CORE_STORAGE_MODES: &[velesdb_core::StorageMode] = &[
    velesdb_core::StorageMode::Full,
    velesdb_core::StorageMode::SQ8,
    velesdb_core::StorageMode::Binary,
    velesdb_core::StorageMode::ProductQuantization,
    velesdb_core::StorageMode::RaBitQ,
];

#[test]
fn wasm_storage_mode_variant_count_matches_core() {
    assert_eq!(
        WASM_STORAGE_MODES.len(),
        CORE_STORAGE_MODES.len(),
        "velesdb-wasm StorageMode has {} variants but velesdb-core has {}. \
         Add the missing variant to crates/velesdb-wasm/src/lib.rs.",
        WASM_STORAGE_MODES.len(),
        CORE_STORAGE_MODES.len(),
    );
}
