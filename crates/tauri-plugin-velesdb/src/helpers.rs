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
        assert!(matches!(
            parse_storage_mode("rabitq"),
            Ok(StorageMode::RaBitQ)
        ));
        // Case-insensitive (delegates to core `StorageMode::from_str`).
        assert!(matches!(
            parse_storage_mode("RaBitQ"),
            Ok(StorageMode::RaBitQ)
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
            StorageMode::RaBitQ,
        ] {
            let s = storage_mode_to_string(mode);
            assert_eq!(parse_storage_mode(s).unwrap(), mode);
        }
    }

    #[cfg(feature = "persistence")]
    #[test]
    fn test_parse_search_quality_none_returns_none() {
        assert!(parse_search_quality(&None)
            .expect("test: should succeed for None")
            .is_none());
    }

    #[cfg(feature = "persistence")]
    #[test]
    fn test_parse_search_quality_named_modes() {
        for mode in ["fast", "balanced", "accurate", "perfect", "auto"] {
            assert!(
                parse_search_quality(&Some(mode.to_string()))
                    .expect("test: named mode should succeed")
                    .is_some(),
                "mode '{mode}' should parse successfully"
            );
        }
    }

    #[cfg(feature = "persistence")]
    #[test]
    fn test_parse_search_quality_custom_and_adaptive() {
        let custom = parse_search_quality(&Some("custom:256".to_string()))
            .expect("test: custom should succeed");
        assert_eq!(custom, Some(velesdb_core::SearchQuality::Custom(256)));

        let adaptive = parse_search_quality(&Some("adaptive:32:512".to_string()))
            .expect("test: adaptive should succeed");
        assert_eq!(
            adaptive,
            Some(velesdb_core::SearchQuality::Adaptive {
                min_ef: 32,
                max_ef: 512,
            })
        );
    }

    #[cfg(feature = "persistence")]
    #[test]
    fn test_parse_search_quality_invalid() {
        assert!(parse_search_quality(&Some("nonexistent".to_string())).is_err());
        assert!(parse_search_quality(&Some(String::new())).is_err());
        assert!(parse_search_quality(&Some("custom:abc".to_string())).is_err());
        assert!(parse_search_quality(&Some("adaptive:512:32".to_string())).is_err());
    }

    // =====================================================================
    // Fusion strategy tests
    // =====================================================================

    #[test]
    fn test_parse_fusion_strategy_valid_strategies() {
        use velesdb_core::fusion::FusionStrategy;

        assert!(matches!(
            parse_fusion_strategy("rrf", None),
            Ok(FusionStrategy::RRF { k: 60 })
        ));
        assert!(matches!(
            parse_fusion_strategy("average", None),
            Ok(FusionStrategy::Average)
        ));
        assert!(matches!(
            parse_fusion_strategy("maximum", None),
            Ok(FusionStrategy::Maximum)
        ));
        assert!(matches!(
            parse_fusion_strategy("weighted", None),
            Ok(FusionStrategy::Weighted { .. })
        ));
        assert!(matches!(
            parse_fusion_strategy("relative_score", None),
            Ok(FusionStrategy::RelativeScore { .. })
        ));
        assert!(matches!(
            parse_fusion_strategy("rsf", None),
            Ok(FusionStrategy::RelativeScore { .. })
        ));
    }

    #[test]
    fn test_parse_fusion_strategy_rrf_custom_k() {
        use velesdb_core::fusion::FusionStrategy;

        let params = serde_json::json!({ "k": 30 });
        let result = parse_fusion_strategy("rrf", Some(&params)).expect("test: valid RRF k");
        assert!(matches!(result, FusionStrategy::RRF { k: 30 }));
    }

    #[test]
    fn test_parse_fusion_strategy_unknown_returns_error() {
        let result = parse_fusion_strategy("nonexistent", None);
        assert!(result.is_err(), "unknown strategy should return error");
    }

    #[test]
    fn test_parse_fusion_strategy_case_insensitive() {
        assert!(parse_fusion_strategy("RRF", None).is_ok());
        assert!(parse_fusion_strategy("Average", None).is_ok());
        assert!(parse_fusion_strategy("MAXIMUM", None).is_ok());
    }

    // =====================================================================
    // require_collection type-check tests
    // =====================================================================

    #[cfg(feature = "persistence")]
    #[test]
    fn test_require_collection_rejects_graph_collection() {
        let tmp = tempfile::TempDir::new().expect("test: create temp dir");
        let db = velesdb_core::Database::open(tmp.path().to_str().expect("test: path"))
            .expect("test: open db");
        db.create_graph_collection("kg", velesdb_core::GraphSchema::schemaless())
            .expect("test: create graph collection");

        let result = require_collection(&db, "kg");
        assert!(
            result.is_err(),
            "require_collection should reject graph collections"
        );
    }
}
