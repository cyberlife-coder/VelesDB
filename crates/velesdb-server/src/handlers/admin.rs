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

use super::helpers::{
    auto_core_error_response, error_response, get_collection_or_404, get_vector_collection_or_404,
};

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
pub async fn get_collection_config(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let config = collection.config();
    let graph_schema = config
        .graph_schema
        .as_ref()
        .and_then(|gs| serde_json::to_value(gs).ok());
    let hnsw_params = config
        .hnsw_params
        .as_ref()
        .and_then(|p| serde_json::to_value(p).ok());

    // `deferred_indexing` is only populated under the `persistence`
    // feature because the core config field is gated the same way.
    #[cfg(feature = "persistence")]
    let deferred_indexing = config
        .deferred_indexing
        .as_ref()
        .and_then(|d| serde_json::to_value(d).ok());
    #[cfg(not(feature = "persistence"))]
    let deferred_indexing = None;

    let async_index_builder = config
        .async_index_builder
        .as_ref()
        .and_then(|a| serde_json::to_value(a).ok());

    Json(CollectionConfigResponse {
        name: config.name,
        dimension: config.dimension,
        metric: format!("{:?}", config.metric).to_lowercase(),
        storage_mode: format!("{:?}", config.storage_mode).to_lowercase(),
        point_count: config.point_count,
        metadata_only: config.metadata_only,
        graph_schema,
        embedding_dimension: config.embedding_dimension,
        schema_version: config.schema_version,
        pq_rescore_oversampling: config.pq_rescore_oversampling,
        hnsw_params,
        deferred_indexing,
        async_index_builder,
    })
    .into_response()
}

/// Rebuilds the HNSW index of a vector collection, reclaiming memory
/// occupied by tombstoned entries and producing a fresh graph from
/// the current vector storage.
///
/// This is a blocking operation: for large collections it may take
/// several seconds. The response includes the number of entries that
/// were compacted during the rebuild.
#[utoipa::path(
    post,
    path = "/collections/{name}/index/rebuild",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Index rebuilt", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Rebuild failed", body = ErrorResponse)
    )
)]
pub async fn rebuild_index(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let result = tokio::task::spawn_blocking(move || collection.rebuild_index()).await;
    match result {
        Ok(Ok(compacted)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Index rebuilt",
                "collection": name,
                "compacted_entries": compacted
            })),
        )
            .into_response(),
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(join_err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("rebuild_index task panicked: {join_err}"),
        ),
    }
}

/// Vacuums the HNSW index of a vector collection, removing tombstoned
/// entries and rebuilding the graph from current vectors.
///
/// Semantically equivalent to `POST /collections/{name}/index/rebuild`
/// but exposed under a more intuitive maintenance-oriented name.
///
/// This is a blocking operation: for large collections it may take
/// several seconds.
#[utoipa::path(
    post,
    path = "/collections/{name}/vacuum",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Index vacuumed", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Vacuum failed", body = ErrorResponse)
    )
)]
pub async fn vacuum_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let result = tokio::task::spawn_blocking(move || collection.rebuild_index()).await;
    match result {
        Ok(Ok(compacted)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Index vacuumed",
                "collection": name,
                "compacted_entries": compacted
            })),
        )
            .into_response(),
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(join_err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("vacuum task panicked: {join_err}"),
        ),
    }
}

/// Compacts the vector storage of a collection, rewriting active vectors
/// into a contiguous layout and reclaiming disk space from deleted entries.
///
/// This is a blocking operation that may involve significant I/O for
/// large, fragmented collections.
#[utoipa::path(
    post,
    path = "/collections/{name}/compact",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Storage compacted", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Compaction failed", body = ErrorResponse)
    )
)]
pub async fn compact_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let result = tokio::task::spawn_blocking(move || collection.compact_storage()).await;
    match result {
        Ok(Ok(bytes_reclaimed)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Storage compacted",
                "collection": name,
                "bytes_reclaimed": bytes_reclaimed
            })),
        )
            .into_response(),
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(join_err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("compact task panicked: {join_err}"),
        ),
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
    let coll_name = name.clone();
    let state_clone = state.clone();
    let result =
        tokio::task::spawn_blocking(move || state_clone.db.analyze_collection(&coll_name)).await;
    match result {
        Ok(Ok(stats)) => {
            let response = map_stats_to_response(&stats);
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(join_err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("analyze_collection task panicked: {join_err}"),
        ),
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
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            format!("No stats for '{name}'. Run POST /collections/{name}/analyze first."),
        ),
        Err(e) => auto_core_error_response(&e),
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
                    histogram_buckets: v.histogram.as_ref().map(|h| h.buckets.len()),
                    histogram_stale: v.histogram.as_ref().map(|h| h.stale),
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
