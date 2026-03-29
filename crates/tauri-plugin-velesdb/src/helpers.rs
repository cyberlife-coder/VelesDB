//! Helper functions for Tauri commands.
//!
//! Centralized parsing and conversion utilities.

#![allow(clippy::missing_errors_doc)] // Internal helpers, errors documented in types

use crate::error::{Error, Result};

/// Parses a metric string into a `DistanceMetric`.
///
/// Delegates to [`DistanceMetric::from_str`](velesdb_core::distance::DistanceMetric::from_str)
/// to keep alias parsing in one place.
pub fn parse_metric(metric: &str) -> Result<velesdb_core::distance::DistanceMetric> {
    metric
        .parse::<velesdb_core::distance::DistanceMetric>()
        .map_err(|e| Error::InvalidConfig(e.to_string()))
}

/// Converts a `DistanceMetric` to its canonical string representation.
///
/// Delegates to [`DistanceMetric::canonical_name`](velesdb_core::distance::DistanceMetric::canonical_name)
/// to keep the mapping in one place.
#[must_use]
pub fn metric_to_string(metric: velesdb_core::distance::DistanceMetric) -> &'static str {
    metric.canonical_name()
}

/// Parses a storage mode string into a `StorageMode`.
///
/// Delegates to [`StorageMode::from_str`] (single source of truth in `velesdb-core`).
pub fn parse_storage_mode(mode: &str) -> Result<velesdb_core::StorageMode> {
    mode.parse::<velesdb_core::StorageMode>()
        .map_err(Error::InvalidConfig)
}

/// Converts a `StorageMode` to its string representation.
///
/// Delegates to [`StorageMode::canonical_name`] (single source of truth in `velesdb-core`).
#[must_use]
pub fn storage_mode_to_string(mode: velesdb_core::StorageMode) -> &'static str {
    mode.canonical_name()
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

/// Looks up a collection by name, returning a typed error on miss.
///
/// Eliminates the repeated `db.get_collection(name).ok_or_else(|| ...)` pattern
/// used by every command that operates on a collection.
///
/// Uses the deprecated `Collection` type for backward compatibility with commands
/// that have not yet migrated to typed collection APIs.
#[allow(deprecated)]
pub fn require_collection(
    db: &velesdb_core::Database,
    name: &str,
) -> Result<velesdb_core::Collection> {
    db.get_collection(name)
        .ok_or_else(|| Error::CollectionNotFound(name.to_string()))
}

/// Looks up a `VectorCollection` by name, returning a typed error on miss.
pub fn require_vector_collection(
    db: &velesdb_core::Database,
    name: &str,
) -> Result<velesdb_core::VectorCollection> {
    db.get_vector_collection(name)
        .ok_or_else(|| Error::CollectionNotFound(name.to_string()))
}

/// Looks up a `GraphCollection` by name, returning a typed error on miss.
pub fn require_graph_collection(
    db: &velesdb_core::Database,
    name: &str,
) -> Result<velesdb_core::GraphCollection> {
    db.get_graph_collection(name)
        .ok_or_else(|| Error::CollectionNotFound(name.to_string()))
}

/// Parses an optional JSON filter value into a core `Filter`.
///
/// Returns `Ok(None)` when the filter is absent.
pub fn parse_filter(filter: &Option<serde_json::Value>) -> Result<Option<velesdb_core::Filter>> {
    match filter {
        Some(filter_json) => {
            let f = velesdb_core::Filter::from_json_value(filter_json.clone())
                .map_err(Error::InvalidConfig)?;
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
