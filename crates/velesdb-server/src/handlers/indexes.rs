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

use super::helpers::{
    auto_core_error_response, error_response, get_vector_collection_or_404, run_blocking,
};

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

    // Index creation scans the collection under write locks — run it on
    // the blocking pool.
    run_blocking(move || create_index_sync(&collection, req))
        .await
        .unwrap_or_else(|resp| resp)
}

/// Synchronous body of [`create_index`]: dispatches the index build and
/// reads back the created index's stats. Runs on the blocking pool.
fn create_index_sync(
    collection: &velesdb_core::collection::VectorCollection,
    req: CreateIndexRequest,
) -> axum::response::Response {
    let result = match dispatch_index_creation(collection, &req) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match result {
        Ok(()) => {
            // Retrieve real cardinality/memory_bytes from the freshly-created index.
            let (cardinality, memory_bytes) = collection
                .list_indexes()
                .into_iter()
                .find(|i| i.label == req.label && i.property == req.property)
                .map_or((0, 0), |i| (i.cardinality, i.memory_bytes));

            (
                StatusCode::CREATED,
                Json(IndexResponse {
                    label: req.label,
                    property: req.property,
                    index_type: req.index_type,
                    cardinality,
                    memory_bytes,
                }),
            )
                .into_response()
        }
        Err(e) => auto_core_error_response(&e),
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

    // Index metadata reads take the index registry lock, which long index
    // builds hold — run it on the blocking pool.
    let core_indexes = match run_blocking(move || collection.list_indexes()).await {
        Ok(indexes) => indexes,
        Err(resp) => return resp,
    };
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

    // Index removal takes the index registry write lock and persists — run
    // it on the blocking pool.
    let dropped_label = label.clone();
    let dropped_property = property.clone();
    let result =
        run_blocking(move || collection.drop_index(&dropped_label, &dropped_property)).await;
    match result {
        Ok(Ok(true)) => Json(serde_json::json!({
            "message": "Index deleted",
            "label": label,
            "property": property
        }))
        .into_response(),
        Ok(Ok(false)) => error_response(
            StatusCode::NOT_FOUND,
            format!("Index on {label}.{property} not found"),
        ),
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(resp) => resp,
    }
}
