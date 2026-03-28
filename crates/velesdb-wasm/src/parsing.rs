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

/// Maps a `velesdb_core::StorageMode` to the local WASM `StorageMode`.
const fn core_to_wasm_storage_mode(core: velesdb_core::StorageMode) -> StorageMode {
    match core {
        velesdb_core::StorageMode::Full => StorageMode::Full,
        velesdb_core::StorageMode::SQ8 => StorageMode::SQ8,
        velesdb_core::StorageMode::Binary => StorageMode::Binary,
        velesdb_core::StorageMode::ProductQuantization => StorageMode::ProductQuantization,
        velesdb_core::StorageMode::RaBitQ => StorageMode::RaBitQ,
        // Reason: StorageMode is #[non_exhaustive] — future variants default to Full.
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
}
