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
pub fn metric_to_string(metric: velesdb_core::distance::DistanceMetric) -> &'static str {
    use velesdb_core::distance::DistanceMetric;
    match metric {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::Euclidean => "euclidean",
        DistanceMetric::DotProduct => "dot",
        DistanceMetric::Hamming => "hamming",
        DistanceMetric::Jaccard => "jaccard",
        // Reason: DistanceMetric is #[non_exhaustive] — future variants default to "unknown".
        _ => "unknown",
    }
}

/// Parses a storage mode string into a `StorageMode`.
pub fn parse_storage_mode(mode: &str) -> Result<velesdb_core::StorageMode> {
    use velesdb_core::StorageMode;
    match mode.to_lowercase().as_str() {
        "full" | "f32" => Ok(StorageMode::Full),
        "sq8" | "int8" => Ok(StorageMode::SQ8),
        "binary" | "bit" => Ok(StorageMode::Binary),
        "pq" | "product_quantization" => Ok(StorageMode::ProductQuantization),
        _ => Err(Error::InvalidConfig(format!(
            "Invalid storage_mode '{mode}'. Use 'full', 'sq8', 'binary', or 'pq'"
        ))),
    }
}

/// Converts a `StorageMode` to its string representation.
#[must_use]
pub fn storage_mode_to_string(mode: velesdb_core::StorageMode) -> &'static str {
    use velesdb_core::StorageMode;
    match mode {
        StorageMode::Full => "full",
        StorageMode::SQ8 => "sq8",
        StorageMode::Binary => "binary",
        StorageMode::ProductQuantization => "pq",
        // Reason: StorageMode is #[non_exhaustive] — future variants default to "unknown".
        _ => "unknown",
    }
}

/// Parses fusion strategy from string and optional params.
#[must_use]
// Reason: JSON f64 → f32 for weights, u64 → u32 for k; values are small config numbers.
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
        unknown => {
            tracing::warn!(
                strategy = unknown,
                "Unknown fusion strategy; falling back to RRF(k=60)"
            );
            FusionStrategy::RRF { k: 60 }
        }
    }
}

/// Parses a sparse vector from JSON string-keyed map to core `SparseVector`.
///
/// JSON only supports string keys, so the frontend sends `{ "42": 0.8, "7": 1.2 }`.
/// This function parses each key to `u32` and constructs a sorted `SparseVector`.
pub fn parse_sparse_vector<S: std::hash::BuildHasher>(
    sparse: &std::collections::HashMap<String, f32, S>,
) -> Result<velesdb_core::sparse_index::SparseVector> {
    let mut pairs = Vec::with_capacity(sparse.len());
    for (key, &value) in sparse {
        let index: u32 = key.parse().map_err(|_| {
            Error::InvalidConfig(format!(
                "Sparse vector key '{key}' is not a valid u32 dimension index"
            ))
        })?;
        pairs.push((index, value));
    }
    Ok(velesdb_core::sparse_index::SparseVector::new(pairs))
}

/// Converts a core `SearchResult` into the Tauri `SearchResult` DTO.
#[must_use]
pub fn map_core_result(r: velesdb_core::SearchResult) -> crate::types::SearchResult {
    crate::types::SearchResult {
        id: r.point.id,
        score: r.score,
        payload: r.point.payload,
    }
}

/// Converts a list of core search results into Tauri `SearchResult` DTOs.
#[must_use]
pub fn map_core_results(
    results: Vec<velesdb_core::SearchResult>,
) -> Vec<crate::types::SearchResult> {
    results.into_iter().map(map_core_result).collect()
}

/// Parses an optional JSON filter value into a core `Filter`.
///
/// Returns `Ok(None)` when the filter is absent.
pub fn parse_filter(filter: &Option<serde_json::Value>) -> Result<Option<velesdb_core::Filter>> {
    match filter {
        Some(filter_json) => {
            let f: velesdb_core::Filter = serde_json::from_value(filter_json.clone())
                .map_err(|e| Error::InvalidConfig(format!("Invalid filter: {e}")))?;
            Ok(Some(f))
        }
        None => Ok(None),
    }
}

/// Wraps search results and a start instant into a `SearchResponse`.
#[must_use]
pub fn timed_search_response(
    results: Vec<crate::types::SearchResult>,
    start: std::time::Instant,
) -> crate::types::SearchResponse {
    crate::types::SearchResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
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
        assert!(matches!(
            parse_storage_mode("pq"),
            Ok(StorageMode::ProductQuantization)
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
            assert_eq!(parse_metric(s).unwrap(), metric);
        }
    }

    #[test]
    fn test_storage_mode_roundtrip() {
        for mode in [
            StorageMode::Full,
            StorageMode::SQ8,
            StorageMode::Binary,
            StorageMode::ProductQuantization,
        ] {
            let s = storage_mode_to_string(mode);
            assert_eq!(parse_storage_mode(s).unwrap(), mode);
        }
    }
}
