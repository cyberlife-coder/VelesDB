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

/// Extracts a named f64 param from JSON, accepting both `camelCase` and `snake_case` keys.
#[allow(clippy::cast_possible_truncation)]
// Reason: JSON f64 → f32 for weights; values are small config numbers (0.0-1.0).
fn extract_weight(
    params: Option<&serde_json::Value>,
    camel: &str,
    snake: &str,
    default: f64,
) -> f32 {
    params
        .and_then(|p| p.get(camel).or_else(|| p.get(snake)))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(default) as f32
}

/// Parses fusion strategy from string and optional params.
///
/// # Errors
///
/// Returns [`Error::InvalidConfig`] if the fusion strategy is unknown or if
/// the RRF `k` parameter exceeds `u32::MAX`.
pub fn parse_fusion_strategy(
    fusion: &str,
    params: Option<&serde_json::Value>,
) -> Result<velesdb_core::fusion::FusionStrategy> {
    use velesdb_core::fusion::FusionStrategy;
    match fusion.to_lowercase().as_str() {
        "rrf" => {
            let raw_k = params
                .and_then(|p| p.get("k"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(60);
            let k = u32::try_from(raw_k).map_err(|_| {
                Error::InvalidConfig(format!("RRF k value {raw_k} exceeds u32 range"))
            })?;
            Ok(FusionStrategy::RRF { k })
        }
        "average" => Ok(FusionStrategy::Average),
        "maximum" => Ok(FusionStrategy::Maximum),
        "weighted" => Ok(FusionStrategy::Weighted {
            avg_weight: extract_weight(params, "avgWeight", "avg_weight", 0.6),
            max_weight: extract_weight(params, "maxWeight", "max_weight", 0.3),
            hit_weight: extract_weight(params, "hitWeight", "hit_weight", 0.1),
        }),
        "relative_score" | "rsf" => Ok(FusionStrategy::RelativeScore {
            dense_weight: extract_weight(params, "denseWeight", "dense_weight", 0.5),
            sparse_weight: extract_weight(params, "sparseWeight", "sparse_weight", 0.5),
        }),
        unknown => Err(Error::InvalidConfig(format!(
            "Unknown fusion strategy: '{unknown}'. \
             Valid strategies: rrf, average, maximum, weighted, relative_score, rsf"
        ))),
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
///
/// `SearchResult` is a type alias for [`velesdb_core::api_types::SearchResultResponse`],
/// so this is a direct field projection from the core search result.
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
/// Returns a `VectorCollection` only if the underlying collection is
/// actually a vector collection. Returns [`Error::InvalidConfig`] if the
/// collection exists but is a graph or metadata collection.
pub fn require_collection(
    db: &velesdb_core::Database,
    name: &str,
) -> Result<velesdb_core::VectorCollection> {
    let any_coll = db
        .get_any_collection(name)
        .ok_or_else(|| Error::CollectionNotFound(name.to_string()))?;
    any_coll.into_vector().map_err(|_other_variant| {
        Error::InvalidConfig(format!("Collection '{name}' is not a vector collection"))
    })
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

/// Parses an optional search quality mode string into a [`SearchQuality`].
///
/// Delegates to [`velesdb_core::api_types::mode_to_search_quality`] to keep
/// mode parsing in one place. Returns `Ok(None)` when the mode is absent.
///
/// [`SearchQuality`]: velesdb_core::SearchQuality
#[cfg(feature = "persistence")]
pub fn parse_search_quality(mode: &Option<String>) -> Result<Option<velesdb_core::SearchQuality>> {
    match mode {
        None => Ok(None),
        Some(m) => velesdb_core::api_types::mode_to_search_quality(m)
            .ok_or_else(|| Error::InvalidConfig(format!("Unknown search quality mode: '{m}'")))
            .map(Some),
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
#[path = "helpers_tests.rs"]
mod tests;
