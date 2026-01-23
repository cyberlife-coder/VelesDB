//! Helper functions for Tauri commands.
//!
//! Centralized parsing and conversion utilities.

#![allow(clippy::missing_errors_doc)] // Internal helpers, errors documented in types

use crate::error::{Error, Result};

/// Parses a metric string into a `DistanceMetric`.
pub fn parse_metric(metric: &str) -> Result<velesdb_core::distance::DistanceMetric> {
    use velesdb_core::distance::DistanceMetric;
    match metric.to_lowercase().as_str() {
        "cosine" => Ok(DistanceMetric::Cosine),
        "euclidean" | "l2" => Ok(DistanceMetric::Euclidean),
        "dot" | "dotproduct" | "inner" => Ok(DistanceMetric::DotProduct),
        "hamming" => Ok(DistanceMetric::Hamming),
        "jaccard" => Ok(DistanceMetric::Jaccard),
        _ => Err(Error::InvalidConfig(format!(
            "Unknown metric '{metric}'. Use: cosine, euclidean, dot, hamming, jaccard"
        ))),
    }
}

/// Converts a `DistanceMetric` to its string representation.
#[must_use]
pub fn metric_to_string(metric: velesdb_core::distance::DistanceMetric) -> String {
    use velesdb_core::distance::DistanceMetric;
    match metric {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::Euclidean => "euclidean",
        DistanceMetric::DotProduct => "dot",
        DistanceMetric::Hamming => "hamming",
        DistanceMetric::Jaccard => "jaccard",
    }
    .to_string()
}

/// Parses a storage mode string into a `StorageMode`.
pub fn parse_storage_mode(mode: &str) -> Result<velesdb_core::StorageMode> {
    use velesdb_core::StorageMode;
    match mode.to_lowercase().as_str() {
        "full" | "f32" => Ok(StorageMode::Full),
        "sq8" | "int8" => Ok(StorageMode::SQ8),
        "binary" | "bit" => Ok(StorageMode::Binary),
        _ => Err(Error::InvalidConfig(format!(
            "Invalid storage_mode '{mode}'. Use 'full', 'sq8', or 'binary'"
        ))),
    }
}

/// Converts a `StorageMode` to its string representation.
#[must_use]
pub fn storage_mode_to_string(mode: velesdb_core::StorageMode) -> String {
    use velesdb_core::StorageMode;
    match mode {
        StorageMode::Full => "full",
        StorageMode::SQ8 => "sq8",
        StorageMode::Binary => "binary",
    }
    .to_string()
}

/// Parses fusion strategy from string and optional params.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn parse_fusion_strategy(
    fusion: &str,
    params: Option<&serde_json::Value>,
) -> velesdb_core::fusion::FusionStrategy {
    use velesdb_core::fusion::FusionStrategy;
    match fusion.to_lowercase().as_str() {
        "rrf" => {
            let k = params
                .and_then(|p| p.get("k"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(60) as u32;
            FusionStrategy::RRF { k }
        }
        "average" => FusionStrategy::Average,
        "maximum" => FusionStrategy::Maximum,
        "weighted" => {
            let avg_weight = params
                .and_then(|p| p.get("avgWeight").or_else(|| p.get("avg_weight")))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.6) as f32;
            let max_weight = params
                .and_then(|p| p.get("maxWeight").or_else(|| p.get("max_weight")))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.3) as f32;
            let hit_weight = params
                .and_then(|p| p.get("hitWeight").or_else(|| p.get("hit_weight")))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.1) as f32;
            FusionStrategy::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            }
        }
        _ => FusionStrategy::RRF { k: 60 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::distance::DistanceMetric;
    use velesdb_core::StorageMode;

    #[test]
    fn test_parse_metric_valid() {
        assert!(matches!(parse_metric("cosine"), Ok(DistanceMetric::Cosine)));
        assert!(matches!(
            parse_metric("EUCLIDEAN"),
            Ok(DistanceMetric::Euclidean)
        ));
        assert!(matches!(parse_metric("l2"), Ok(DistanceMetric::Euclidean)));
        assert!(matches!(
            parse_metric("dot"),
            Ok(DistanceMetric::DotProduct)
        ));
    }

    #[test]
    fn test_parse_metric_invalid() {
        assert!(parse_metric("unknown").is_err());
    }

    #[test]
    fn test_parse_storage_mode_valid() {
        assert!(matches!(parse_storage_mode("full"), Ok(StorageMode::Full)));
        assert!(matches!(parse_storage_mode("sq8"), Ok(StorageMode::SQ8)));
        assert!(matches!(
            parse_storage_mode("binary"),
            Ok(StorageMode::Binary)
        ));
    }

    #[test]
    fn test_metric_roundtrip() {
        for metric in [
            DistanceMetric::Cosine,
            DistanceMetric::Euclidean,
            DistanceMetric::DotProduct,
            DistanceMetric::Hamming,
            DistanceMetric::Jaccard,
        ] {
            let s = metric_to_string(metric);
            assert_eq!(parse_metric(&s).unwrap(), metric);
        }
    }

    #[test]
    fn test_storage_mode_roundtrip() {
        for mode in [StorageMode::Full, StorageMode::SQ8, StorageMode::Binary] {
            let s = storage_mode_to_string(mode);
            assert_eq!(parse_storage_mode(&s).unwrap(), mode);
        }
    }
}
