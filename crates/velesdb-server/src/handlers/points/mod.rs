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

use crate::types::{ErrorResponse, SparseVectorInput, UpsertPointsRequest};
use crate::AppState;
use velesdb_core::Point;

use crate::handlers::helpers::{error_response, get_vector_collection_or_404};

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
        Ok(Err(e)) => error_response(StatusCode::BAD_REQUEST, e.to_string()),
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
        None => error_response(StatusCode::NOT_FOUND, format!("Point {} not found", id)),
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
        Err(e) => error_response(StatusCode::BAD_REQUEST, e.to_string()),
    }
}
