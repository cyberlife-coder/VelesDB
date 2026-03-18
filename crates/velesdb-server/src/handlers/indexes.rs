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

use super::helpers::{error_response, get_vector_collection_or_404};

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
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let result = match dispatch_index_creation(&collection, &req) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match result {
        Ok(()) => (
            StatusCode::CREATED,
            Json(IndexResponse {
                label: req.label,
                property: req.property,
                index_type: req.index_type,
                cardinality: 0,
                memory_bytes: 0,
            }),
        )
            .into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

/// Dispatch index creation by type.
#[allow(clippy::result_large_err)]
fn dispatch_index_creation(
    collection: &velesdb_core::collection::VectorCollection,
    req: &CreateIndexRequest,
) -> Result<velesdb_core::error::Result<()>, axum::response::Response> {
    match req.index_type.to_lowercase().as_str() {
        "hash" => Ok(collection.create_property_index(&req.label, &req.property)),
        "range" => Ok(collection.create_range_index(&req.label, &req.property)),
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            format!("Invalid index_type: {}. Valid: hash, range", req.index_type),
        )),
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
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let core_indexes = collection.list_indexes();
    let indexes: Vec<IndexResponse> = core_indexes
        .into_iter()
        .map(|i| IndexResponse {
            label: i.label,
            property: i.property,
            index_type: i.index_type,
            cardinality: i.cardinality,
            memory_bytes: i.memory_bytes,
        })
        .collect();
    let total = indexes.len();

    Json(ListIndexesResponse { indexes, total }).into_response()
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
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match collection.drop_index(&label, &property) {
        Ok(dropped) => {
            if dropped {
                Json(serde_json::json!({
                    "message": "Index deleted",
                    "label": label,
                    "property": property
                }))
                .into_response()
            } else {
                error_response(
                    StatusCode::NOT_FOUND,
                    format!("Index on {}.{} not found", label, property),
                )
            }
        }
        Err(e) => error_response(StatusCode::BAD_REQUEST, e.to_string()),
    }
}
