//! Collection management handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{CollectionResponse, CreateCollectionRequest, ErrorResponse};
use crate::AppState;
use velesdb_core::{DistanceMetric, StorageMode};

use super::helpers::{core_error_response, error_response, get_collection_or_404};

/// List all collections.
#[utoipa::path(
    get,
    path = "/collections",
    tag = "collections",
    responses(
        (status = 200, description = "List of collections", body = Object)
    )
)]
pub async fn list_collections(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let collections = state.db.list_collections();
    Json(serde_json::json!({ "collections": collections }))
}

/// Create a new collection.
#[utoipa::path(
    post,
    path = "/collections",
    tag = "collections",
    request_body = CreateCollectionRequest,
    responses(
        (status = 201, description = "Collection created", body = Object),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
pub async fn create_collection(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCollectionRequest>,
) -> impl IntoResponse {
    let metric = match parse_distance_metric(&req.metric) {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    let storage_mode = match parse_storage_mode(&req.storage_mode) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let result = match dispatch_create(&state, &req, metric, storage_mode) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match result {
        Ok(()) => create_collection_success_response(&req),
        Err(e) => core_error_response(StatusCode::BAD_REQUEST, &e),
    }
}

/// Parse a distance metric string into the core enum.
///
/// Delegates to [`DistanceMetric::from_str`] to keep alias parsing in one place.
#[allow(clippy::result_large_err)]
fn parse_distance_metric(raw: &str) -> Result<DistanceMetric, axum::response::Response> {
    raw.parse::<DistanceMetric>()
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, e.to_string()))
}

/// Parse a storage mode string into the core enum.
///
/// Delegates to [`StorageMode::from_str`] (single source of truth in `velesdb-core`).
#[allow(clippy::result_large_err)]
fn parse_storage_mode(raw: &str) -> Result<StorageMode, axum::response::Response> {
    raw.parse::<StorageMode>()
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, e))
}

/// Create a vector collection, requiring a dimension in the request.
#[allow(clippy::result_large_err)]
fn create_vector_collection(
    state: &AppState,
    req: &CreateCollectionRequest,
    metric: DistanceMetric,
    storage_mode: StorageMode,
) -> Result<velesdb_core::error::Result<()>, axum::response::Response> {
    let dimension = req.dimension.ok_or_else(|| {
        error_response(
            StatusCode::BAD_REQUEST,
            "dimension is required for vector collections".to_string(),
        )
    })?;
    if req.hnsw_m.is_some() || req.hnsw_ef_construction.is_some() {
        Ok(state.db.create_vector_collection_with_hnsw(
            &req.name,
            dimension,
            metric,
            storage_mode,
            req.hnsw_m,
            req.hnsw_ef_construction,
        ))
    } else {
        Ok(state.db.create_vector_collection_with_options(
            &req.name,
            dimension,
            metric,
            storage_mode,
        ))
    }
}

/// Dispatch collection creation based on `collection_type`.
#[allow(clippy::result_large_err)]
fn dispatch_create(
    state: &AppState,
    req: &CreateCollectionRequest,
    metric: DistanceMetric,
    storage_mode: StorageMode,
) -> Result<velesdb_core::error::Result<()>, axum::response::Response> {
    match req.collection_type.to_lowercase().as_str() {
        "metadata_only" | "metadata-only" | "metadata" => {
            Ok(state.db.create_metadata_collection(&req.name))
        }
        "graph" | "knowledge_graph" | "kg" => {
            use velesdb_core::GraphSchema;
            Ok(state
                .db
                .create_graph_collection(&req.name, GraphSchema::schemaless()))
        }
        "vector" | "" => create_vector_collection(state, req, metric, storage_mode),
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Invalid collection_type: {}. Valid: vector, graph, metadata_only",
                req.collection_type
            ),
        )),
    }
}

/// Build a 201 Created response for successful collection creation.
fn create_collection_success_response(req: &CreateCollectionRequest) -> axum::response::Response {
    let mut warnings = Vec::new();
    let is_vector = matches!(req.collection_type.to_lowercase().as_str(), "vector" | "");
    if is_vector {
        warnings.push("Collection dimension and metric are immutable after creation. If your embedding model changes, create a new collection and reindex data.");
        warnings.push("For first queries, start without strict filters/thresholds, then tighten progressively.");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "message": "Collection created",
            "name": req.name,
            "type": req.collection_type,
            "warnings": warnings
        })),
    )
        .into_response()
}

/// Get collection information.
#[utoipa::path(
    get,
    path = "/collections/{name}",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection details", body = CollectionResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn get_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let config = collection.config();
    Json(CollectionResponse {
        name: config.name,
        dimension: config.dimension,
        metric: format!("{:?}", config.metric).to_lowercase(),
        point_count: config.point_count,
        storage_mode: format!("{:?}", config.storage_mode).to_lowercase(),
    })
    .into_response()
}

/// Run a quick sanity check for onboarding and troubleshooting.
#[utoipa::path(
    get,
    path = "/collections/{name}/sanity",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection sanity status", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn collection_sanity(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let config = collection.config();
    build_sanity_response(&state, &config, &collection)
}

/// Build the JSON sanity check response body.
#[allow(deprecated)]
fn build_sanity_response(
    state: &AppState,
    config: &velesdb_core::collection::CollectionConfig,
    collection: &velesdb_core::Collection,
) -> axum::response::Response {
    let has_data = config.point_count > 0;
    Json(serde_json::json!({
        "collection": config.name,
        "dimension": config.dimension,
        "metric": format!("{:?}", config.metric).to_lowercase(),
        "point_count": config.point_count,
        "is_empty": collection.is_empty(),
        "checks": {
            "has_vectors": has_data,
            "search_ready": has_data,
            "dimension_configured": config.dimension > 0
        },
        "diagnostics": {
            "search_requests_total": state.onboarding_metrics.search_requests_total.load(std::sync::atomic::Ordering::Relaxed),
            "dimension_mismatch_total": state.onboarding_metrics.dimension_mismatch_total.load(std::sync::atomic::Ordering::Relaxed),
            "empty_search_results_total": state.onboarding_metrics.empty_search_results_total.load(std::sync::atomic::Ordering::Relaxed),
            "filter_parse_errors_total": state.onboarding_metrics.filter_parse_errors_total.load(std::sync::atomic::Ordering::Relaxed)
        },
        "hints": if has_data {
            vec![
                "Run a search without strict filters first, then tighten filters progressively."
            ]
        } else {
            vec![
                "Insert at least one known vector before evaluating search quality.",
                "Verify you are querying the intended collection."
            ]
        }
    }))
    .into_response()
}

/// Delete a collection.
#[utoipa::path(
    delete,
    path = "/collections/{name}",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection deleted", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn delete_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.delete_collection(&name) {
        Ok(()) => Json(serde_json::json!({
            "message": "Collection deleted",
            "name": name
        }))
        .into_response(),
        Err(e) => core_error_response(StatusCode::NOT_FOUND, &e),
    }
}

/// Check if a collection is empty.
#[utoipa::path(
    get,
    path = "/collections/{name}/empty",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Empty status", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn is_empty(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    Json(serde_json::json!({
        "is_empty": collection.is_empty()
    }))
    .into_response()
}

/// Flush pending changes to disk.
#[utoipa::path(
    post,
    path = "/collections/{name}/flush",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Flushed successfully", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Flush failed", body = ErrorResponse)
    )
)]
pub async fn flush_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match collection.flush() {
        Ok(()) => Json(serde_json::json!({
            "message": "Flushed successfully",
            "collection": name
        }))
        .into_response(),
        Err(e) => core_error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}
