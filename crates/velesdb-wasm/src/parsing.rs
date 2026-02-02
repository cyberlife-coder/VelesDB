//! Parsing helpers for WASM bindings.
//!
//! Centralizes metric and storage mode parsing to avoid duplication.
//! Uses String errors internally for testability, converted to JsValue at call site.

use wasm_bindgen::prelude::*;

use velesdb_core::DistanceMetric;
use crate::StorageMode;

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
    match metric.to_lowercase().as_str() {
        "cosine" => Ok(DistanceMetric::Cosine),
        "euclidean" | "l2" => Ok(DistanceMetric::Euclidean),
        "dot" | "dotproduct" | "inner" => Ok(DistanceMetric::DotProduct),
        "hamming" => Ok(DistanceMetric::Hamming),
        "jaccard" => Ok(DistanceMetric::Jaccard),
        _ => Err("Unknown metric. Use: cosine, euclidean, dot, hamming, jaccard".to_string()),
    }
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

fn parse_storage_mode_inner(mode: &str) -> Result<StorageMode, String> {
    match mode.to_lowercase().as_str() {
        "full" => Ok(StorageMode::Full),
        "sq8" => Ok(StorageMode::SQ8),
        "binary" => Ok(StorageMode::Binary),
        _ => Err("Unknown storage mode. Use: full, sq8, binary".to_string()),
    }
}

/// Validates that a vector has the expected dimension.
///
/// # Errors
/// Returns a JsValue error with a descriptive message if dimensions don't match.
pub fn validate_dimension(
    actual: usize,
    expected: usize,
    context: &str,
) -> Result<(), JsValue> {
    if actual != expected {
        Err(JsValue::from_str(&format!(
            "{} dimension mismatch: expected {}, got {}",
            context, expected, actual
        )))
    } else {
        Ok(())
    }
}

/// Converts a metric byte to DistanceMetric (for import).
///
/// # Errors
/// Returns a JsValue error if the byte is invalid.
pub fn metric_from_byte(byte: u8) -> Result<DistanceMetric, JsValue> {
    metric_from_byte_inner(byte).map_err(|e| JsValue::from_str(&e))
}

fn metric_from_byte_inner(byte: u8) -> Result<DistanceMetric, String> {
    match byte {
        0 => Ok(DistanceMetric::Cosine),
        1 => Ok(DistanceMetric::Euclidean),
        2 => Ok(DistanceMetric::DotProduct),
        3 => Ok(DistanceMetric::Hamming),
        4 => Ok(DistanceMetric::Jaccard),
        _ => Err("Invalid metric byte".to_string()),
    }
}

/// Converts a DistanceMetric to byte (for export).
#[must_use]
pub const fn metric_to_byte(metric: DistanceMetric) -> u8 {
    match metric {
        DistanceMetric::Cosine => 0,
        DistanceMetric::Euclidean => 1,
        DistanceMetric::DotProduct => 2,
        DistanceMetric::Hamming => 3,
        DistanceMetric::Jaccard => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_metric_valid() {
        assert!(matches!(parse_metric_inner("cosine"), Ok(DistanceMetric::Cosine)));
        assert!(matches!(parse_metric_inner("EUCLIDEAN"), Ok(DistanceMetric::Euclidean)));
        assert!(matches!(parse_metric_inner("l2"), Ok(DistanceMetric::Euclidean)));
        assert!(matches!(parse_metric_inner("dot"), Ok(DistanceMetric::DotProduct)));
        assert!(matches!(parse_metric_inner("dotproduct"), Ok(DistanceMetric::DotProduct)));
        assert!(matches!(parse_metric_inner("hamming"), Ok(DistanceMetric::Hamming)));
        assert!(matches!(parse_metric_inner("jaccard"), Ok(DistanceMetric::Jaccard)));
    }

    #[test]
    fn test_parse_metric_invalid() {
        assert!(parse_metric_inner("unknown").is_err());
    }

    #[test]
    fn test_parse_storage_mode_valid() {
        assert!(matches!(parse_storage_mode_inner("full"), Ok(StorageMode::Full)));
        assert!(matches!(parse_storage_mode_inner("SQ8"), Ok(StorageMode::SQ8)));
        assert!(matches!(parse_storage_mode_inner("binary"), Ok(StorageMode::Binary)));
    }

    #[test]
    fn test_parse_storage_mode_invalid() {
        assert!(parse_storage_mode_inner("unknown").is_err());
    }

    #[test]
    fn test_metric_byte_roundtrip() {
        for metric in [
            DistanceMetric::Cosine,
            DistanceMetric::Euclidean,
            DistanceMetric::DotProduct,
            DistanceMetric::Hamming,
            DistanceMetric::Jaccard,
        ] {
            let byte = metric_to_byte(metric);
            assert_eq!(metric_from_byte_inner(byte).unwrap(), metric);
        }
    }
}
