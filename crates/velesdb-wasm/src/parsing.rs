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

fn parse_metric_inner(metric: &str) -> Result<DistanceMetric, String> {
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
fn parse_storage_mode_inner(mode: &str) -> Result<StorageMode, String> {
    let core: velesdb_core::StorageMode = mode.parse()?;
    Ok(core_to_wasm_storage_mode(core))
}

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
const fn core_to_wasm_storage_mode(core: velesdb_core::StorageMode) -> StorageMode {
    match core {
        velesdb_core::StorageMode::Full => StorageMode::Full,
        velesdb_core::StorageMode::SQ8 => StorageMode::SQ8,
        velesdb_core::StorageMode::Binary => StorageMode::Binary,
        velesdb_core::StorageMode::ProductQuantization => StorageMode::ProductQuantization,
        velesdb_core::StorageMode::RaBitQ => StorageMode::RaBitQ,
        // FIXME(PRE-SEED): New StorageMode variants silently map to Full. Update when core adds variants.
        _ => StorageMode::Full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_metric_valid() {
        assert!(matches!(
            parse_metric_inner("cosine"),
            Ok(DistanceMetric::Cosine)
        ));
        assert!(matches!(
            parse_metric_inner("EUCLIDEAN"),
            Ok(DistanceMetric::Euclidean)
        ));
        assert!(matches!(
            parse_metric_inner("l2"),
            Ok(DistanceMetric::Euclidean)
        ));
        assert!(matches!(
            parse_metric_inner("dot"),
            Ok(DistanceMetric::DotProduct)
        ));
        assert!(matches!(
            parse_metric_inner("dotproduct"),
            Ok(DistanceMetric::DotProduct)
        ));
        assert!(matches!(
            parse_metric_inner("hamming"),
            Ok(DistanceMetric::Hamming)
        ));
        assert!(matches!(
            parse_metric_inner("jaccard"),
            Ok(DistanceMetric::Jaccard)
        ));
    }

    #[test]
    fn test_parse_metric_invalid() {
        assert!(parse_metric_inner("unknown").is_err());
    }

    #[test]
    fn test_metric_parsing_is_delegated_to_core_source_of_truth() {
        use std::str::FromStr;

        for alias in ["cosine", "l2", "dot", "inner", "hamming", "jaccard"] {
            let parsed = parse_metric_inner(alias).unwrap();
            let from_core = DistanceMetric::from_str(alias).unwrap();
            assert_eq!(parsed, from_core);
        }
    }

    #[test]
    fn test_parse_storage_mode_valid() {
        assert!(matches!(
            parse_storage_mode_inner("full"),
            Ok(StorageMode::Full)
        ));
        assert!(matches!(
            parse_storage_mode_inner("SQ8"),
            Ok(StorageMode::SQ8)
        ));
        assert!(matches!(
            parse_storage_mode_inner("binary"),
            Ok(StorageMode::Binary)
        ));
        assert!(matches!(
            parse_storage_mode_inner("pq"),
            Ok(StorageMode::ProductQuantization)
        ));
    }

    #[test]
    fn test_parse_storage_mode_invalid() {
        assert!(parse_storage_mode_inner("unknown").is_err());
    }

    // =========================================================================
    // SearchQuality parsing tests
    // =========================================================================

    #[test]
    fn test_parse_search_quality_named_modes() {
        assert!(parse_search_quality_inner("fast").is_ok());
        assert!(parse_search_quality_inner("balanced").is_ok());
        assert!(parse_search_quality_inner("accurate").is_ok());
        assert!(parse_search_quality_inner("perfect").is_ok());
        assert!(parse_search_quality_inner("autotune").is_ok());
        assert!(parse_search_quality_inner("auto").is_ok());
    }

    #[test]
    fn test_parse_search_quality_case_insensitive() {
        assert!(parse_search_quality_inner("FAST").is_ok());
        assert!(parse_search_quality_inner("Balanced").is_ok());
        assert!(parse_search_quality_inner("AUTOTUNE").is_ok());
    }

    #[test]
    fn test_parse_search_quality_custom() {
        assert!(parse_search_quality_inner("custom:256").is_ok());
    }

    #[test]
    fn test_parse_search_quality_custom_case_insensitive() {
        assert!(parse_search_quality_inner("Custom:128").is_ok());
    }

    #[test]
    fn test_parse_search_quality_custom_invalid() {
        let err = parse_search_quality_inner("custom:abc");
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("Invalid custom ef_search"));
    }

    #[test]
    fn test_parse_search_quality_adaptive() {
        assert!(parse_search_quality_inner("adaptive:32:512").is_ok());
    }

    #[test]
    fn test_parse_search_quality_adaptive_equal_bounds() {
        assert!(parse_search_quality_inner("adaptive:100:100").is_ok());
    }

    #[test]
    fn test_parse_search_quality_adaptive_inverted_range() {
        let err = parse_search_quality_inner("adaptive:512:32");
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("must be <= max_ef"));
    }

    #[test]
    fn test_parse_search_quality_adaptive_missing_max() {
        let err = parse_search_quality_inner("adaptive:32");
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("Invalid adaptive format"));
    }

    #[test]
    fn test_parse_search_quality_unknown() {
        let err = parse_search_quality_inner("nonexistent");
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("Unknown search quality"));
    }
}
