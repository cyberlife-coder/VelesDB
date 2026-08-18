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
/// - "full" - Full f32 precision
/// - "sq8" - 8-bit scalar quantization
/// - "binary" - 1-bit quantization
///
/// # Errors
/// Returns a JsValue error if the mode is not recognized.
pub fn parse_storage_mode(mode: &str) -> Result<StorageMode, JsValue> {
    parse_storage_mode_inner(mode).map_err(|e| JsValue::from_str(&e))
}

/// Delegates to [`velesdb_core::StorageMode::from_str`] (single source of truth)
/// and maps to the local WASM `StorageMode` enum.
///
/// # PQ / `RaBitQ` fallback
///
/// The browser engine has no Product-Quantization codebook training (that
/// path is `persistence`-gated in core and unavailable in WASM). A request
/// for `"pq"` or `"rabitq"` is therefore stored using the SQ8 encode/decode
/// path. This is architecturally justified, but no longer silent: a one-time
/// `console.warn` is emitted so the caller knows the requested mode was
/// downgraded.
fn parse_storage_mode_inner(mode: &str) -> Result<StorageMode, String> {
    let core: velesdb_core::StorageMode = mode.parse()?;
    warn_if_pq_fallback(core);
    Ok(core_to_wasm_storage_mode(core))
}

/// Emits a one-time `console.warn` when PQ/`RaBitQ` is requested in WASM,
/// where it transparently falls back to the SQ8 storage path.
///
/// The `console.warn` binding requires a live JS environment, so it is only
/// invoked on the `wasm32` target. On native (test) builds this is a no-op.
#[cfg(target_arch = "wasm32")]
fn warn_if_pq_fallback(core: velesdb_core::StorageMode) {
    if matches!(
        core,
        velesdb_core::StorageMode::ProductQuantization | velesdb_core::StorageMode::RaBitQ
    ) {
        web_sys::console::warn_1(&JsValue::from_str(
            "VelesDB WASM: Product Quantization / RaBitQ is not available in the \
             browser engine (no codebook training); storing vectors with SQ8 \
             quantization instead.",
        ));
    }
}

/// Native no-op counterpart of [`warn_if_pq_fallback`]; see the `wasm32`
/// variant for the browser behavior.
#[cfg(not(target_arch = "wasm32"))]
fn warn_if_pq_fallback(_core: velesdb_core::StorageMode) {}

/// Validates a search quality string for API parity with Python and Server SDKs.
///
/// In WASM, search is brute-force O(n) — there is no HNSW graph, so
/// `ef_search` has no effect. This function validates the quality string
/// (rejecting unknown modes) for forward-compatibility. The core
/// `SearchQuality` enum is behind the `persistence` feature gate, so we
/// validate locally without depending on it.
///
/// # Supported values
///
/// - `"fast"`, `"balanced"`, `"accurate"`, `"perfect"`, `"autotune"` / `"auto"`
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
    let lower = mode.to_lowercase();
    match lower.as_str() {
        "fast" | "balanced" | "accurate" | "perfect" | "autotune" | "auto_tune" | "auto" => Ok(()),
        other => parse_advanced_quality(other),
    }
}

/// Validates `custom:<ef>` and `adaptive:<min_ef>:<max_ef>` quality modes.
fn parse_advanced_quality(mode: &str) -> Result<(), String> {
    if let Some(ef_str) = mode.strip_prefix("custom:") {
        ef_str.parse::<usize>().map_err(|_| {
            format!(
                "Invalid custom ef_search value: '{ef_str}'. Expected integer, \
                 e.g. 'custom:256'"
            )
        })?;
        return Ok(());
    }
    if let Some(params) = mode.strip_prefix("adaptive:") {
        return parse_adaptive_params(params);
    }
    Err(format!(
        "Unknown search quality: '{mode}'. Valid: fast, balanced, accurate, perfect, \
         autotune, custom:<ef>, adaptive:<min_ef>:<max_ef>"
    ))
}

/// Validates `<min_ef>:<max_ef>` for the adaptive quality mode.
fn parse_adaptive_params(params: &str) -> Result<(), String> {
    let parts: Vec<&str> = params.split(':').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid adaptive format: '{params}'. Expected 'adaptive:<min_ef>:<max_ef>'"
        ));
    }
    let min_ef = parts[0]
        .parse::<usize>()
        .map_err(|_| format!("Invalid adaptive min_ef: '{}'", parts[0]))?;
    let max_ef = parts[1]
        .parse::<usize>()
        .map_err(|_| format!("Invalid adaptive max_ef: '{}'", parts[1]))?;
    if min_ef > max_ef {
        return Err(format!(
            "Adaptive min_ef ({min_ef}) must be <= max_ef ({max_ef})"
        ));
    }
    Ok(())
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
