//! Admin and diagnostic handlers: stats, config, guardrails, analyze.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{
    CollectionConfigResponse, CollectionStatsResponse, ColumnStatsResponse, ErrorResponse,
    GuardRailsConfigRequest, GuardRailsConfigResponse, IndexStatsResponse,
};
use crate::AppState;

/// Get detailed collection configuration (HNSW params, storage mode, schema, etc.).
#[utoipa::path(
    get,
    path = "/collections/{name}/config",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection configuration", body = CollectionConfigResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
#[allow(deprecated)]
pub async fn get_collection_config(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.get_collection(&name) {
        Some(collection) => {
            let config = collection.config();
            let graph_schema = config
                .graph_schema
                .as_ref()
                .and_then(|gs| serde_json::to_value(gs).ok());

            Json(CollectionConfigResponse {
                name: config.name,
                dimension: config.dimension,
                metric: format!("{:?}", config.metric).to_lowercase(),
                storage_mode: format!("{:?}", config.storage_mode).to_lowercase(),
                point_count: config.point_count,
                metadata_only: config.metadata_only,
                graph_schema,
                embedding_dimension: config.embedding_dimension,
            })
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{}' not found", name),
            }),
        )
            .into_response(),
    }
}

/// Analyze a collection, computing and persisting statistics.
#[utoipa::path(
    post,
    path = "/collections/{name}/analyze",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection analyzed", body = CollectionStatsResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Analysis failed", body = ErrorResponse)
    )
)]
pub async fn analyze_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.analyze_collection(&name) {
        Ok(stats) => {
            let response = map_stats_to_response(&stats);
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}

/// Get cached collection statistics (returns 404 if never analyzed).
#[utoipa::path(
    get,
    path = "/collections/{name}/stats",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection statistics", body = CollectionStatsResponse),
        (status = 404, description = "No statistics available", body = ErrorResponse),
        (status = 500, description = "Failed to read stats", body = ErrorResponse)
    )
)]
pub async fn get_collection_stats(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.get_collection_stats(&name) {
        Ok(Some(stats)) => {
            let response = map_stats_to_response(&stats);
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!(
                    "No stats for '{name}'. Run POST /collections/{name}/analyze first."
                ),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Get current guard-rails configuration.
#[utoipa::path(
    get,
    path = "/guardrails",
    tag = "guardrails",
    responses(
        (status = 200, description = "Current guard-rails config", body = GuardRailsConfigResponse)
    )
)]
pub async fn get_guardrails(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let limits = state.query_limits.read();
    Json(limits_to_response(&limits))
}

/// Update guard-rails configuration (partial update).
#[utoipa::path(
    put,
    path = "/guardrails",
    tag = "guardrails",
    request_body = GuardRailsConfigRequest,
    responses(
        (status = 200, description = "Updated guard-rails config", body = GuardRailsConfigResponse)
    )
)]
pub async fn update_guardrails(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GuardRailsConfigRequest>,
) -> impl IntoResponse {
    let mut limits = state.query_limits.write();
    apply_guardrails_update(&mut limits, &req);

    // Propagate the updated limits to all active collections so that
    // subsequent queries use the new thresholds (EPIC-048).
    state.db.update_guardrails(&limits);

    Json(limits_to_response(&limits))
}

/// Convert `QueryLimits` to the REST response type.
fn limits_to_response(limits: &velesdb_core::guardrails::QueryLimits) -> GuardRailsConfigResponse {
    GuardRailsConfigResponse {
        max_depth: limits.max_depth,
        max_cardinality: limits.max_cardinality,
        memory_limit_bytes: limits.memory_limit_bytes,
        timeout_ms: limits.timeout_ms,
        rate_limit_qps: limits.rate_limit_qps,
        circuit_failure_threshold: limits.circuit_failure_threshold,
        circuit_recovery_seconds: limits.circuit_recovery_seconds,
    }
}

/// Convert core `CollectionStats` to the REST response type.
fn map_stats_to_response(
    stats: &velesdb_core::collection::stats::CollectionStats,
) -> CollectionStatsResponse {
    let column_stats = stats
        .column_stats
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                ColumnStatsResponse {
                    name: v.name.clone(),
                    null_count: v.null_count,
                    distinct_count: v.distinct_count,
                    min_value: v.min_value.clone(),
                    max_value: v.max_value.clone(),
                    avg_size_bytes: v.avg_size_bytes,
                },
            )
        })
        .collect();

    let index_stats = stats
        .index_stats
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                IndexStatsResponse {
                    name: v.name.clone(),
                    index_type: v.index_type.clone(),
                    entry_count: v.entry_count,
                    depth: v.depth,
                    size_bytes: v.size_bytes,
                },
            )
        })
        .collect();

    CollectionStatsResponse {
        total_points: stats.total_points,
        total_size_bytes: stats.total_size_bytes,
        row_count: stats.row_count,
        deleted_count: stats.deleted_count,
        avg_row_size_bytes: stats.avg_row_size_bytes,
        payload_size_bytes: stats.payload_size_bytes,
        last_analyzed_epoch_ms: stats.last_analyzed_epoch_ms,
        column_stats,
        index_stats,
    }
}

/// Apply partial update fields to query limits.
fn apply_guardrails_update(
    limits: &mut velesdb_core::guardrails::QueryLimits,
    req: &GuardRailsConfigRequest,
) {
    if let Some(v) = req.max_depth {
        limits.max_depth = v;
    }
    if let Some(v) = req.max_cardinality {
        limits.max_cardinality = v;
    }
    if let Some(v) = req.memory_limit_bytes {
        limits.memory_limit_bytes = v;
    }
    if let Some(v) = req.timeout_ms {
        limits.timeout_ms = v;
    }
    if let Some(v) = req.rate_limit_qps {
        limits.rate_limit_qps = v;
    }
    if let Some(v) = req.circuit_failure_threshold {
        limits.circuit_failure_threshold = v;
    }
    if let Some(v) = req.circuit_recovery_seconds {
        limits.circuit_recovery_seconds = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::guardrails::QueryLimits;

    #[test]
    fn test_limits_to_response_roundtrip() {
        let limits = QueryLimits::default();
        let response = limits_to_response(&limits);
        assert_eq!(response.max_depth, limits.max_depth);
        assert_eq!(response.max_cardinality, limits.max_cardinality);
        assert_eq!(response.memory_limit_bytes, limits.memory_limit_bytes);
        assert_eq!(response.timeout_ms, limits.timeout_ms);
        assert_eq!(response.rate_limit_qps, limits.rate_limit_qps);
        assert_eq!(
            response.circuit_failure_threshold,
            limits.circuit_failure_threshold
        );
        assert_eq!(
            response.circuit_recovery_seconds,
            limits.circuit_recovery_seconds
        );
    }

    #[test]
    fn test_apply_guardrails_partial_update() {
        let mut limits = QueryLimits::default();
        let original_timeout = limits.timeout_ms;

        let req = GuardRailsConfigRequest {
            max_depth: Some(20),
            max_cardinality: None,
            memory_limit_bytes: None,
            timeout_ms: None,
            rate_limit_qps: Some(500),
            circuit_failure_threshold: None,
            circuit_recovery_seconds: None,
        };

        apply_guardrails_update(&mut limits, &req);

        assert_eq!(limits.max_depth, 20);
        assert_eq!(limits.rate_limit_qps, 500);
        // Unchanged fields remain at defaults
        assert_eq!(limits.timeout_ms, original_timeout);
    }

    #[test]
    fn test_apply_guardrails_full_update() {
        let mut limits = QueryLimits::default();

        let req = GuardRailsConfigRequest {
            max_depth: Some(5),
            max_cardinality: Some(50_000),
            memory_limit_bytes: Some(1024 * 1024),
            timeout_ms: Some(10_000),
            rate_limit_qps: Some(200),
            circuit_failure_threshold: Some(3),
            circuit_recovery_seconds: Some(60),
        };

        apply_guardrails_update(&mut limits, &req);

        assert_eq!(limits.max_depth, 5);
        assert_eq!(limits.max_cardinality, 50_000);
        assert_eq!(limits.memory_limit_bytes, 1024 * 1024);
        assert_eq!(limits.timeout_ms, 10_000);
        assert_eq!(limits.rate_limit_qps, 200);
        assert_eq!(limits.circuit_failure_threshold, 3);
        assert_eq!(limits.circuit_recovery_seconds, 60);
    }

    #[test]
    fn test_guardrails_response_serialization() {
        let response = GuardRailsConfigResponse {
            max_depth: 10,
            max_cardinality: 100_000,
            memory_limit_bytes: 104_857_600,
            timeout_ms: 30_000,
            rate_limit_qps: 100,
            circuit_failure_threshold: 5,
            circuit_recovery_seconds: 30,
        };
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains("\"max_depth\":10"));
        assert!(json.contains("\"rate_limit_qps\":100"));
    }
}
