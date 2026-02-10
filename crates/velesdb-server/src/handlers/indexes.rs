//! Index management handlers (EPIC-009 Propagation).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{CreateIndexRequest, ErrorResponse, IndexResponse, ListIndexesResponse};
use crate::AppState;

/// Create a property index on a graph collection.
#[utoipa::path(
    post,
    path = "/collections/{name}/indexes",
    tag = "indexes",
    request_body = CreateIndexRequest,
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 201, description = "Index created", body = IndexResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn create_index(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<CreateIndexRequest>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    // Validate index_type before spawn_blocking
    let index_type_lower = req.index_type.to_lowercase();
    if !matches!(index_type_lower.as_str(), "hash" | "range") {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid index_type: {}. Valid: hash, range", req.index_type),
            }),
        )
            .into_response();
    }

    let label = req.label;
    let property = req.property;
    let itype = req.index_type;
    let l = label.clone();
    let p = property.clone();

    let result = tokio::task::spawn_blocking(move || {
        if index_type_lower == "hash" {
            collection.create_property_index(&l, &p)
        } else {
            collection.create_range_index(&l, &p)
        }
    })
    .await;

    match result {
        Ok(Ok(())) => (
            StatusCode::CREATED,
            Json(IndexResponse {
                label,
                property,
                index_type: itype,
                cardinality: 0,
                memory_bytes: 0,
            }),
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

/// List all indexes on a collection.
#[utoipa::path(
    get,
    path = "/collections/{name}/indexes",
    tag = "indexes",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "List of indexes", body = ListIndexesResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn list_indexes(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    let result = tokio::task::spawn_blocking(move || {
        let core_indexes = collection.list_indexes();
        core_indexes
            .into_iter()
            .map(|i| IndexResponse {
                label: i.label,
                property: i.property,
                index_type: i.index_type,
                cardinality: i.cardinality,
                memory_bytes: i.memory_bytes,
            })
            .collect::<Vec<_>>()
    })
    .await;

    match result {
        Ok(indexes) => {
            let total = indexes.len();
            Json(ListIndexesResponse { indexes, total }).into_response()
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

/// Delete a property index.
#[utoipa::path(
    delete,
    path = "/collections/{name}/indexes/{label}/{property}",
    tag = "indexes",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("label" = String, Path, description = "Node label"),
        ("property" = String, Path, description = "Property name")
    ),
    responses(
        (status = 200, description = "Index deleted", body = Object),
        (status = 404, description = "Index or collection not found", body = ErrorResponse)
    )
)]
pub async fn delete_index(
    State(state): State<Arc<AppState>>,
    Path((name, label, property)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    let l = label.clone();
    let p = property.clone();
    let result = tokio::task::spawn_blocking(move || collection.drop_index(&l, &p)).await;

    match result {
        Ok(Ok(true)) => Json(serde_json::json!({
            "message": "Index deleted",
            "label": label,
            "property": property
        }))
        .into_response(),
        Ok(Ok(false)) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Index on {}.{} not found", label, property),
            }),
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
