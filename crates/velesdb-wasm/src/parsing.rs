//! Parsing helpers for WASM bindings.
//!
//! Centralizes metric and storage mode parsing to avoid duplication.
//! Uses String errors internally for testability, converted to JsValue at call site.

use wasm_bindgen::prelude::*;

use crate::StorageMode;
use velesdb_core::DistanceMetric;

/// Parses a metric string into a DistanceMetric.
///
/// # Supported values
/// - "cosine"
/// - "euclidean", "l2"
/// - "dot", "dotproduct", "inner"
/// - "hamming"
/// - "jaccard"
///
/// # Errors
/// Returns a JsValue error if the metric is not recognized.
pub fn parse_metric(metric: &str) -> Result<DistanceMetric, JsValue> {
    parse_metric_inner(metric).map_err(|e| JsValue::from_str(&e))
}

pub(crate) fn parse_metric_inner(metric: &str) -> Result<DistanceMetric, String> {
    use std::str::FromStr;

    DistanceMetric::from_str(metric).map_err(std::string::ToString::to_string)
}

/// Parses a storage mode string into a StorageMode.
///
/// # Supported values
/// The names and aliases `velesdb-core` accepts: `full`, `sq8`, `binary`,
/// `pq`, `rabitq`. Every mode keeps f32 vectors and returns the same results
/// as core with that mode (see [`StorageMode`]).
///
/// # Errors
/// Returns a JsValue error if the mode is not recognized.
pub fn parse_storage_mode(mode: &str) -> Result<StorageMode, JsValue> {
    parse_storage_mode_inner(mode).map_err(|e| JsValue::from_str(&e))
}

/// Delegates to [`velesdb_core::StorageMode::from_str`] (single source of truth)
/// and maps to the local WASM `StorageMode` enum.
fn parse_storage_mode_inner(mode: &str) -> Result<StorageMode, String> {
    let core: velesdb_core::StorageMode = mode.parse()?;
    Ok(core_to_wasm_storage_mode(core))
}

/// Validates a search quality string for API parity with Python and Server SDKs.
///
/// In WASM, search is brute-force O(n) — there is no HNSW graph, so
/// `ef_search` has no effect. This function validates the quality string
/// (rejecting unknown modes) for forward-compatibility, through core's own
/// grammar, [`velesdb_core::api_types::parse_search_mode`]: the forms, the
/// aliases, the ef range (#2275) and the messages are the ones every other
/// surface reports, and a form core gains reaches WASM with it.
///
/// # Supported values
///
/// - `"fast"`, `"balanced"`, `"accurate"`, `"perfect"`,
///   `"autotune"` / `"auto_tune"` / `"auto"`
/// - `"custom:<ef>"` (e.g. `"custom:256"`)
/// - `"adaptive:<min_ef>:<max_ef>"` (e.g. `"adaptive:32:512"`)
///
/// # Errors
///
/// Returns a `JsValue` error if the quality string is not recognized.
pub fn parse_search_quality(quality: &str) -> Result<(), JsValue> {
    parse_search_quality_inner(quality).map_err(|e| JsValue::from_str(&e))
}

/// Inner parser returning `String` errors for testability.
///
/// Returns `Ok(())` when the quality string is valid.
fn parse_search_quality_inner(mode: &str) -> Result<(), String> {
    velesdb_core::api_types::parse_search_mode(mode).map(drop)
}

/// Maps a `velesdb_core::StorageMode` to the local WASM `StorageMode`.
///
/// `velesdb_core::StorageMode` is `#[non_exhaustive]`, so the catch-all is
/// required by the compiler. The fallback to `StorageMode::Full` keeps the
/// WASM bundle resilient when a future core variant has not yet been
/// propagated. In debug builds, the `debug_assert!` makes the gap visible
/// immediately during testing — see `tests::core_to_wasm_storage_mode_*`
/// for the contract.
fn core_to_wasm_storage_mode(core: velesdb_core::StorageMode) -> StorageMode {
    match core {
        velesdb_core::StorageMode::Full => StorageMode::Full,
        velesdb_core::StorageMode::SQ8 => StorageMode::SQ8,
        velesdb_core::StorageMode::Binary => StorageMode::Binary,
        velesdb_core::StorageMode::ProductQuantization => StorageMode::ProductQuantization,
        velesdb_core::StorageMode::RaBitQ => StorageMode::RaBitQ,
        other => {
            debug_assert!(
                false,
                "core StorageMode variant `{}` not propagated to WASM; \
                 update core_to_wasm_storage_mode and the round-trip test",
                other.canonical_name()
            );
            StorageMode::Full
        }
    }
}

#[cfg(test)]
#[path = "parsing_tests.rs"]
mod tests;
