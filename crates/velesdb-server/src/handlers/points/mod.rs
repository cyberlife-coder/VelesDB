//! Point operations handlers.

pub mod streaming;

pub use streaming::{
    __path_stream_insert, __path_stream_upsert_points, stream_insert, stream_upsert_points,
};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{
    ErrorResponse, ScrollPoint, ScrollRequest, ScrollResponse, SparseVectorInput,
    UpsertPointsRequest,
};
use crate::AppState;
use velesdb_core::Point;

use crate::handlers::helpers::{
    auto_core_error_response, error_response, get_vector_collection_or_404,
};

use velesdb_core::index::sparse::SparseVector;

/// Converts sparse vector input fields from a request into a `BTreeMap<String, SparseVector>`.
///
/// Merges `sparse_vector` (single, stored under `""`) and `sparse_vectors` (named map).
/// Named map takes precedence if both provide the same key.
fn convert_sparse_inputs(
    sparse_vector: Option<SparseVectorInput>,
    sparse_vectors: Option<std::collections::BTreeMap<String, SparseVectorInput>>,
) -> Result<Option<std::collections::BTreeMap<String, SparseVector>>, String> {
    let has_single = sparse_vector.is_some();
    let has_named = sparse_vectors.as_ref().is_some_and(|m| !m.is_empty());

    if !has_single && !has_named {
        return Ok(None);
    }

    let mut result = std::collections::BTreeMap::new();

    // Single sparse vector goes under default name ""
    if let Some(sv_input) = sparse_vector {
        let sv = sv_input.into_sparse_vector()?;
        result.insert(String::new(), sv);
    }

    // Named sparse vectors (overwrite default if same key).
    // If both `sparse_vector` and `sparse_vectors[""]` are supplied, the named map wins.
    // A debug trace is emitted so operators can detect this (usually unintentional) pattern.
    if let Some(named) = sparse_vectors {
        for (name, sv_input) in named {
            let sv = sv_input
                .into_sparse_vector()
                .map_err(|e| format!("sparse_vectors['{name}']: {e}"))?;
            if name.is_empty() && result.contains_key("") {
                tracing::debug!(
                    "sparse_vector (default \"\") is being overwritten by \
                     sparse_vectors[\"\"] — supply only one to avoid ambiguity"
                );
            }
            result.insert(name, sv);
        }
    }

    Ok(Some(result))
}

/// Upsert points to a collection.
#[utoipa::path(
    post,
    path = "/collections/{name}/points",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = UpsertPointsRequest,
    responses(
        (status = 200, description = "Points upserted", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
pub async fn upsert_points(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<UpsertPointsRequest>,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let points = match build_points_from_request(req) {
        Ok(p) => p,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, e);
        }
    };

    // CRITICAL: upsert_bulk is blocking (HNSW insertion + I/O).
    // Must use spawn_blocking to avoid blocking the async runtime.
    let result = tokio::task::spawn_blocking(move || collection.upsert_bulk(&points)).await;

    match result {
        Ok(Ok(inserted)) => {
            state.db.notify_upsert(&name, inserted);
            Json(serde_json::json!({
                "message": "Points upserted",
                "count": inserted
            }))
            .into_response()
        }
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task panicked: {e}"),
        ),
    }
}

/// Convert an `UpsertPointsRequest` into a `Vec<Point>`, merging sparse inputs.
fn build_points_from_request(req: UpsertPointsRequest) -> Result<Vec<Point>, String> {
    let mut points: Vec<Point> = Vec::with_capacity(req.points.len());
    for p in req.points {
        let sparse = convert_sparse_inputs(p.sparse_vector, p.sparse_vectors)?;
        let mut point = Point::new(p.id, p.vector, p.payload);
        point.sparse_vectors = sparse;
        points.push(point);
    }
    Ok(points)
}

/// Get a point by ID.
#[utoipa::path(
    get,
    path = "/collections/{name}/points/{id}",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("id" = u64, Path, description = "Point ID")
    ),
    responses(
        (status = 200, description = "Point found", body = Object),
        (status = 404, description = "Point or collection not found", body = ErrorResponse)
    )
)]
pub async fn get_point(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, u64)>,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let points = collection.get(&[id]);

    match points.into_iter().next().flatten() {
        Some(point) => Json(serde_json::json!({
            "id": point.id,
            "vector": point.vector,
            "payload": point.payload
        }))
        .into_response(),
        // PR #586 Devin fix: emit `VELES-003 PointNotFound` via
        // `auto_core_error_response` so typed-error clients surface
        // `PointNotFoundError` instead of a generic fallback.
        None => auto_core_error_response(&velesdb_core::Error::PointNotFound(id)),
    }
}

/// Delete a point by ID.
#[utoipa::path(
    delete,
    path = "/collections/{name}/points/{id}",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("id" = u64, Path, description = "Point ID")
    ),
    responses(
        (status = 200, description = "Point deleted", body = Object),
        (status = 404, description = "Point or collection not found", body = ErrorResponse)
    )
)]
pub async fn delete_point(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, u64)>,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match collection.delete(&[id]) {
        Ok(()) => Json(serde_json::json!({
            "message": "Point deleted",
            "id": id
        }))
        .into_response(),
        Err(e) => auto_core_error_response(&e),
    }
}

/// Maximum allowed batch size for scroll requests.
const MAX_SCROLL_BATCH_SIZE: u32 = 10_000;

/// Scroll through collection points with cursor-based pagination.
#[utoipa::path(
    post,
    path = "/collections/{name}/points/scroll",
    tag = "points",
    params(("name" = String, Path, description = "Collection name")),
    request_body = ScrollRequest,
    responses(
        (status = 200, description = "Scroll batch", body = ScrollResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn scroll_points(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<ScrollRequest>,
) -> impl IntoResponse {
    if req.batch_size == 0 || req.batch_size > MAX_SCROLL_BATCH_SIZE {
        return error_response(
            StatusCode::BAD_REQUEST,
            "batch_size must be between 1 and 10000".to_string(),
        );
    }

    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let filter = match parse_scroll_filter(&req.filter) {
        Ok(f) => f,
        Err(resp) => return resp,
    };

    let batch_size = req.batch_size as usize;
    let cursor = req.cursor;

    // scroll_batch is blocking (reads from storage).
    let result = tokio::task::spawn_blocking(move || {
        collection.scroll_batch(cursor, batch_size, filter.as_ref())
    })
    .await;

    match result {
        Ok(Ok(batch)) => build_scroll_response(batch),
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task panicked: {e}"),
        ),
    }
}

/// Parse the optional filter JSON into a core `Filter`.
#[allow(clippy::result_large_err)]
fn parse_scroll_filter(
    filter_json: &Option<serde_json::Value>,
) -> Result<Option<velesdb_core::Filter>, axum::response::Response> {
    let Some(ref json) = filter_json else {
        return Ok(None);
    };
    serde_json::from_value::<velesdb_core::Filter>(json.clone())
        .map(Some)
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, format!("Invalid filter: {e}")))
}

/// Convert a core `ScrollBatch` into an HTTP JSON response.
fn build_scroll_response(batch: velesdb_core::ScrollBatch) -> axum::response::Response {
    let points: Vec<ScrollPoint> = batch
        .points
        .into_iter()
        .map(|p| ScrollPoint {
            id: p.id,
            vector: p.vector,
            payload: p.payload,
        })
        .collect();
    Json(ScrollResponse {
        next_cursor: batch.next_cursor,
        points,
    })
    .into_response()
}

/// Maximum number of IDs in a single bulk delete request.
const MAX_BULK_DELETE_SIZE: usize = 10_000;

/// Request body for bulk point deletion.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct BulkDeleteRequest {
    /// List of point IDs to delete.
    pub ids: Vec<u64>,
}

/// Deletes multiple points by ID in a single request.
///
/// Accepts a JSON body with a list of point IDs. All IDs are passed to
/// the underlying `Collection::delete(&[u64])` in one call, which is
/// more efficient than individual deletions.
///
/// Returns the number of points that were requested for deletion.
/// Points that do not exist are silently skipped.
#[utoipa::path(
    post,
    path = "/collections/{name}/points/delete",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = BulkDeleteRequest,
    responses(
        (status = 200, description = "Points deleted", body = Object),
        (status = 400, description = "Batch too large", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Delete failed", body = ErrorResponse)
    )
)]
pub async fn bulk_delete_points(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<BulkDeleteRequest>,
) -> impl IntoResponse {
    if req.ids.is_empty() {
        return Json(serde_json::json!({
            "message": "No points to delete",
            "collection": name,
            "deleted_count": 0
        }))
        .into_response();
    }

    if req.ids.len() > MAX_BULK_DELETE_SIZE {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Batch too large: {} IDs (max {MAX_BULK_DELETE_SIZE})",
                req.ids.len()
            ),
        );
    }

    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let ids = req.ids;
    let count = ids.len();
    let coll_name = name.clone();

    let result = tokio::task::spawn_blocking(move || collection.delete(&ids)).await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({
            "message": "Points deleted",
            "collection": coll_name,
            "deleted_count": count
        }))
        .into_response(),
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(join_err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("bulk_delete task panicked: {join_err}"),
        ),
    }
}
