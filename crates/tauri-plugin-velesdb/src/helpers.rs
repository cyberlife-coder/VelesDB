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

/// Extracts a named f64 param from JSON, accepting both `camelCase` and
/// `snake_case` keys; `None` when absent, so core supplies the default.
#[allow(clippy::cast_possible_truncation)]
// Reason: JSON f64 → f32 for weights; values are small config numbers (0.0-1.0).
fn extract_weight(params: Option<&serde_json::Value>, camel: &str, snake: &str) -> Option<f32> {
    params
        .and_then(|p| p.get(camel).or_else(|| p.get(snake)))
        .and_then(serde_json::Value::as_f64)
        .map(|weight| weight as f32)
}

/// Extracts a named f64 array param from JSON — `weighted_rrf`'s per-branch
/// `weights`, the one fusion parameter that is not a scalar. Accepts the
/// `weights`/`branch_weights`/`branchWeights` keys; `None` when absent, so
/// core reports the missing weights itself.
#[allow(clippy::cast_possible_truncation)]
// Reason: JSON f64 → f32 for weights; values are small config numbers (0.0-1.0).
fn extract_weights(params: Option<&serde_json::Value>) -> Option<Vec<f32>> {
    ["weights", "branch_weights", "branchWeights"]
        .iter()
        .find_map(|key| params.and_then(|p| p.get(key)))
        .and_then(serde_json::Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .map(|item| item.as_f64().map(|weight| weight as f32))
                .collect()
        })
}

/// Parses fusion strategy from string and optional params.
///
/// The name, its aliases, the case rule and the default of every parameter
/// `params` leaves out are core's grammar
/// ([`velesdb_core::FusionStrategy::from_name_with_weights`], which also
/// carries `weighted_rrf`'s per-branch weights), so a strategy reads the same
/// here as on every other surface, and its weights are validated when it is
/// built.
///
/// # Errors
///
/// Returns [`Error::InvalidConfig`] if the fusion strategy is unknown, if its
/// weights are refused, or if the RRF `k` parameter exceeds `u32::MAX`.
pub fn parse_fusion_strategy(
    fusion: &str,
    params: Option<&serde_json::Value>,
) -> Result<velesdb_core::fusion::FusionStrategy> {
    let raw_k = params
        .and_then(|p| p.get("k"))
        .and_then(serde_json::Value::as_u64);
    let rrf_k = raw_k
        .map(|raw| {
            u32::try_from(raw)
                .map_err(|_| Error::InvalidConfig(format!("RRF k value {raw} exceeds u32 range")))
        })
        .transpose()?;
    let fusion_params = velesdb_core::FusionParams::default()
        .with_rrf_k(rrf_k)
        .with_avg_weight(extract_weight(params, "avgWeight", "avg_weight"))
        .with_max_weight(extract_weight(params, "maxWeight", "max_weight"))
        .with_hit_weight(extract_weight(params, "hitWeight", "hit_weight"))
        .with_dense_weight(extract_weight(params, "denseWeight", "dense_weight"))
        .with_sparse_weight(extract_weight(params, "sparseWeight", "sparse_weight"));
    velesdb_core::FusionStrategy::from_name_with_weights(
        fusion,
        fusion_params,
        extract_weights(params).as_deref(),
    )
    .map_err(|e| Error::InvalidConfig(e.to_string()))
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
/// Every known variant is accepted: graph and metadata collections are
/// re-wrapped through `into_vector_view`, the purely structural re-wrap
/// the Python binding uses for the same purpose — so data-plane commands
/// (points, scroll, flush, guardrails, …) work on any creatable
/// collection. Vector-semantic operations stay guarded by the engine
/// itself (a metadata collection reports dimension 0, so a dense query
/// still fails its dimension check). An unknown future variant keeps
/// the previous rejection shape.
pub fn require_collection(
    db: &velesdb_core::Database,
    name: &str,
) -> Result<velesdb_core::VectorCollection> {
    let any_coll = db
        .get_any_collection(name)
        .ok_or_else(|| Error::CollectionNotFound(name.to_string()))?;
    Ok(match any_coll {
        velesdb_core::AnyCollection::Vector(c) => c,
        velesdb_core::AnyCollection::Graph(g) => g.into_vector_view(),
        velesdb_core::AnyCollection::Metadata(m) => m.into_vector_view(),
        // `AnyCollection` is `#[non_exhaustive]`; a variant added later has
        // no vector view to offer.
        _ => {
            return Err(Error::InvalidConfig(format!(
                "Collection '{name}' is not a supported collection kind"
            )));
        }
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
/// Delegates to [`velesdb_core::api_types::parse_search_mode`], so a mode the
/// server refuses is refused here too, with the same message naming the
/// accepted forms. Returns `Ok(None)` when the mode is absent.
///
/// [`SearchQuality`]: velesdb_core::SearchQuality
pub fn parse_search_quality(mode: &Option<String>) -> Result<Option<velesdb_core::SearchQuality>> {
    mode.as_deref()
        .map(velesdb_core::api_types::parse_search_mode)
        .transpose()
        .map_err(Error::InvalidConfig)
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

/// Consults the database's read gate for `name` before a collection read:
/// `Err` is the observer's denial, `Ok(Some(filter))` a scope to AND into the
/// read, `Ok(None)` a plain allow. With no observer this is a single `Option`
/// check.
pub(crate) fn authorize_collection_read(
    db: &velesdb_core::Database,
    name: &str,
    operation: velesdb_core::observer::QueryOperationKind,
) -> Result<Option<velesdb_core::Filter>> {
    db.authorize_read(name, operation, None, None)
        .map_err(Error::Database)
}

/// Fails a read whose shape cannot carry an observer scope filter (fail
/// closed), the same refusal every other uncomposable read gives.
pub(crate) fn deny_if_scoped(scope: Option<&velesdb_core::Filter>, context: &str) -> Result<()> {
    if scope.is_some() {
        return Err(Error::InvalidConfig(format!(
            "scope narrowing is not supported for {context}"
        )));
    }
    Ok(())
}

/// AND-composes a caller filter with an observer scope filter, so the scope
/// can only narrow what the caller asked for.
pub(crate) fn and_scope(
    caller: Option<velesdb_core::Filter>,
    scope: Option<velesdb_core::Filter>,
) -> Option<velesdb_core::Filter> {
    match (caller, scope) {
        (None, None) => None,
        (Some(c), None) => Some(c),
        (None, Some(s)) => Some(s),
        (Some(c), Some(s)) => Some(velesdb_core::Filter::new(velesdb_core::Condition::And {
            conditions: vec![c.condition, s.condition],
        })),
    }
}

/// Decides a read that cannot carry an observer scope: the denial propagates
/// and a scope refuses (fail closed) — the metadata/graph-read shape.
pub(crate) fn require_unscoped_read(
    db: &velesdb_core::Database,
    name: &str,
    operation: velesdb_core::observer::QueryOperationKind,
    context: &str,
) -> Result<()> {
    deny_if_scoped(
        authorize_collection_read(db, name, operation)?.as_ref(),
        context,
    )
}
