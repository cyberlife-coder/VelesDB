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
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid storage_mode: {}. Valid: full, sq8, binary",
                        req.storage_mode
                    ),
                }),
            )
                .into_response()
        }
    };

    let result = match req.collection_type.to_lowercase().as_str() {
        "metadata_only" | "metadata-only" => {
            use velesdb_core::CollectionType;
            state
                .db
                .create_collection_typed(&req.name, &CollectionType::MetadataOnly)
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
            state
                .db
                .create_collection_with_options(&req.name, dimension, metric, storage_mode)
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid collection_type: {}. Valid: vector, metadata_only",
                        req.collection_type
                    ),
                }),
            )
                .into_response()
        }
    };

    match result {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "message": "Collection created",
                "name": req.name,
                "type": req.collection_type
            })),
        )
            .into_response(),
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
