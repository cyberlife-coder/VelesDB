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
    let result = tokio::task::spawn_blocking(move || state.db.list_collections()).await;
    match result {
        Ok(collections) => Json(serde_json::json!({ "collections": collections })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
            }),
        )
            .into_response(),
    }
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
#[allow(clippy::too_many_lines)]
// Reason: validation of 3 enums (metric, storage, type) + spawn_blocking makes this inherently long
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

    let collection_type_str = req.collection_type.to_lowercase();
    let dimension = req.dimension;
    let name = req.name;
    let ctype = req.collection_type;

    match collection_type_str.as_str() {
        "metadata_only" | "metadata-only" | "vector" | "" => {}
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid collection_type: {}. Valid: vector, metadata_only",
                        ctype
                    ),
                }),
            )
                .into_response()
        }
    }

    if matches!(collection_type_str.as_str(), "vector" | "") && dimension.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "dimension is required for vector collections".to_string(),
            }),
        )
            .into_response();
    }

    let name_for_task = name.clone();
    let result = tokio::task::spawn_blocking(move || {
        if matches!(
            collection_type_str.as_str(),
            "metadata_only" | "metadata-only"
        ) {
            use velesdb_core::CollectionType;
            state
                .db
                .create_collection_typed(&name_for_task, &CollectionType::MetadataOnly)
        } else {
            // Reason: dimension validated as Some above for vector type
            let dim = dimension.expect("validated above");
            state
                .db
                .create_collection_with_options(&name_for_task, dim, metric, storage_mode)
        }
    })
    .await;

    match result {
        Ok(Ok(())) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "message": "Collection created",
                "name": name,
                "type": ctype
            })),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
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
    let result = tokio::task::spawn_blocking(move || {
        state.db.get_collection(&name).map(|collection| {
            let config = collection.config();
            (name, config)
        })
    })
    .await;

    match result {
        Ok(Some((_, config))) => Json(CollectionResponse {
            name: config.name,
            dimension: config.dimension,
            metric: format!("{:?}", config.metric).to_lowercase(),
            point_count: config.point_count,
            storage_mode: format!("{:?}", config.storage_mode).to_lowercase(),
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Collection not found".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
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
    let n = name.clone();
    let result = tokio::task::spawn_blocking(move || state.db.delete_collection(&n)).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({
            "message": "Collection deleted",
            "name": name
        }))
        .into_response(),
        Ok(Err(e)) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
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
    let result = tokio::task::spawn_blocking(move || {
        state
            .db
            .get_collection(&name)
            .map(|c| c.is_empty())
            .ok_or(name)
    })
    .await;

    match result {
        Ok(Ok(empty)) => Json(serde_json::json!({ "is_empty": empty })).into_response(),
        Ok(Err(name)) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{}' not found", name),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
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
    let n = name.clone();
    let result = tokio::task::spawn_blocking(move || {
        let collection = state.db.get_collection(&n).ok_or(n)?;
        collection.flush().map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({
            "message": "Flushed successfully",
            "collection": name
        }))
        .into_response(),
        Ok(Err(err_or_name)) => {
            // Reason: if get_collection fails, err_or_name is the collection name
            if err_or_name.contains(' ') {
                // It's a flush error message
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Flush failed: {}", err_or_name),
                    }),
                )
                    .into_response()
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: format!("Collection '{}' not found", err_or_name),
                    }),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
            }),
        )
            .into_response(),
    }
}
