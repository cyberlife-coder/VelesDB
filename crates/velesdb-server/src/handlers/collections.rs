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
    let metric = match req.metric.to_lowercase().as_str() {
        "cosine" => DistanceMetric::Cosine,
        "euclidean" | "l2" => DistanceMetric::Euclidean,
        "dot" | "dotproduct" | "ip" => DistanceMetric::DotProduct,
        "hamming" => DistanceMetric::Hamming,
        "jaccard" => DistanceMetric::Jaccard,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid metric: {}. Valid: cosine, euclidean, dot, hamming, jaccard",
                        req.metric
                    ),
                }),
            )
                .into_response()
        }
    };

    let storage_mode = match req.storage_mode.to_lowercase().as_str() {
        "full" | "f32" => StorageMode::Full,
        "sq8" | "int8" => StorageMode::SQ8,
        "binary" | "bit" => StorageMode::Binary,
        "pq" | "product_quantization" => StorageMode::ProductQuantization,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid storage_mode: {}. Valid: full, sq8, binary, pq",
                        req.storage_mode
                    ),
                }),
            )
                .into_response()
        }
    };

    let result = match req.collection_type.to_lowercase().as_str() {
        "metadata_only" | "metadata-only" | "metadata" => {
            state.db.create_metadata_collection(&req.name)
        }
        "graph" | "knowledge_graph" | "kg" => {
            use velesdb_core::GraphSchema;
            state
                .db
                .create_graph_collection(&req.name, GraphSchema::schemaless())
        }
        "vector" | "" => {
            let dimension = match req.dimension {
                Some(d) => d,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "dimension is required for vector collections".to_string(),
                        }),
                    )
                        .into_response()
                }
            };
            if req.hnsw_m.is_some() || req.hnsw_ef_construction.is_some() {
                state.db.create_vector_collection_with_hnsw(
                    &req.name,
                    dimension,
                    metric,
                    storage_mode,
                    req.hnsw_m,
                    req.hnsw_ef_construction,
                )
            } else {
                state.db.create_vector_collection_with_options(
                    &req.name,
                    dimension,
                    metric,
                    storage_mode,
                )
            }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid collection_type: {}. Valid: vector, graph, metadata_only",
                        req.collection_type
                    ),
                }),
            )
                .into_response()
        }
    };

    match result {
        Ok(()) => {
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
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
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
    match state.db.get_collection(&name) {
        Some(collection) => {
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
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{}' not found", name),
            }),
        )
            .into_response(),
    }
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
    match state.db.get_collection(&name) {
        Some(collection) => {
            let config = collection.config();
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
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{}' not found", name),
            }),
        )
            .into_response(),
    }
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
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
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
    match state.db.get_collection(&name) {
        Some(collection) => Json(serde_json::json!({
            "is_empty": collection.is_empty()
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{}' not found", name),
            }),
        )
            .into_response(),
    }
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
    match state.db.get_collection(&name) {
        Some(collection) => match collection.flush() {
            Ok(()) => Json(serde_json::json!({
                "message": "Flushed successfully",
                "collection": name
            }))
            .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Flush failed: {}", e),
                }),
            )
                .into_response(),
        },
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{}' not found", name),
            }),
        )
            .into_response(),
    }
}
