//! Point operations handlers.

use axum::{
    body::Body,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use futures::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::types::{ErrorResponse, SparseVectorInput, StreamInsertRequest, UpsertPointsRequest};
use crate::AppState;
use velesdb_core::{BackpressureError, Point, VectorCollection};

use velesdb_core::index::sparse::SparseVector;

const STREAM_BATCH_SIZE: usize = 100;
const STREAM_BATCH_MAX_WAIT: Duration = Duration::from_millis(100);

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

#[derive(Default)]
struct StreamUpsertStats {
    inserted: usize,
    malformed: usize,
    failed_upserts: usize,
}

fn parse_ndjson_line(line: &str, batch: &mut Vec<Point>, stats: &mut StreamUpsertStats) {
    if line.is_empty() {
        return;
    }

    match serde_json::from_str::<Point>(line) {
        Ok(point) => batch.push(point),
        Err(error) => {
            stats.malformed += 1;
            tracing::warn!(error = %error, "Skipping malformed NDJSON point");
        }
    }
}

async fn flush_point_batch(
    collection: VectorCollection,
    batch: &mut Vec<Point>,
    stats: &mut StreamUpsertStats,
) {
    if batch.is_empty() {
        return;
    }

    let points = std::mem::take(batch);
    let batch_size = points.len();

    match tokio::task::spawn_blocking(move || collection.upsert_bulk(&points)).await {
        Ok(Ok(inserted)) => stats.inserted += inserted,
        Ok(Err(error)) => {
            stats.failed_upserts += batch_size;
            tracing::error!(
                error = %error,
                batch_size,
                "Failed to upsert streamed batch"
            );
        }
        Err(error) => {
            stats.failed_upserts += batch_size;
            tracing::error!(
                error = %error,
                batch_size,
                "Stream upsert batch task panicked"
            );
        }
    }
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
    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!(
                        "Collection '{}' not found or is not a vector collection",
                        name
                    ),
                }),
            )
                .into_response()
        }
    };

    let mut points: Vec<Point> = Vec::with_capacity(req.points.len());
    for p in req.points {
        let sparse = match convert_sparse_inputs(p.sparse_vector, p.sparse_vectors) {
            Ok(s) => s,
            Err(e) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
            }
        };
        let mut point = Point::new(p.id, p.vector, p.payload);
        point.sparse_vectors = sparse;
        points.push(point);
    }

    // CRITICAL: upsert_bulk is blocking (HNSW insertion + I/O)
    // Must use spawn_blocking to avoid blocking the async runtime
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

/// Stream upsert points using NDJSON.
#[utoipa::path(
    post,
    path = "/collections/{name}/points/stream",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body(content = String, content_type = "application/x-ndjson", description = "NDJSON stream with one point per line"),
    responses(
        (status = 200, description = "Stream processed", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn stream_upsert_points(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Body,
) -> impl IntoResponse {
    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!(
                        "Collection '{}' not found or is not a vector collection",
                        name
                    ),
                }),
            )
                .into_response()
        }
    };

    let mut stream = body.into_data_stream();
    let mut buffer = Vec::<u8>::new();
    let mut batch = Vec::with_capacity(STREAM_BATCH_SIZE);
    let mut stats = StreamUpsertStats::default();
    let mut last_flush = Instant::now();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                buffer.extend_from_slice(&chunk);

                while let Some(newline_pos) = buffer.iter().position(|byte| *byte == b'\n') {
                    let line_bytes: Vec<u8> = buffer.drain(..=newline_pos).collect();
                    let line = String::from_utf8_lossy(&line_bytes);
                    parse_ndjson_line(line.trim(), &mut batch, &mut stats);

                    if batch.len() >= STREAM_BATCH_SIZE
                        || (!batch.is_empty() && last_flush.elapsed() >= STREAM_BATCH_MAX_WAIT)
                    {
                        flush_point_batch(collection.clone(), &mut batch, &mut stats).await;
                        last_flush = Instant::now();
                    }
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "Error while reading request body stream");
            }
        }
    }

    if !buffer.is_empty() {
        let line = String::from_utf8_lossy(&buffer);
        parse_ndjson_line(line.trim(), &mut batch, &mut stats);
    }

    flush_point_batch(collection, &mut batch, &mut stats).await;

    if stats.inserted > 0 {
        state.db.notify_upsert(&name, stats.inserted);
    }

    Json(serde_json::json!({
        "message": "Stream processed",
        "inserted": stats.inserted,
        "malformed": stats.malformed,
        "failed_upserts": stats.failed_upserts
    }))
    .into_response()
}

/// Stream-insert a single point via the bounded ingestion channel.
///
/// Returns 202 Accepted on success, 429 Too Many Requests when the buffer is
/// full (with `Retry-After: 0.1` header), and 404 when the collection is not
/// found.
#[utoipa::path(
    post,
    path = "/collections/{name}/stream/insert",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = StreamInsertRequest,
    responses(
        (status = 202, description = "Point accepted into streaming buffer"),
        (status = 429, description = "Streaming buffer full — retry after delay", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 409, description = "Streaming not configured", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn stream_insert(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<StreamInsertRequest>,
) -> impl IntoResponse {
    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!(
                        "Collection '{}' not found or is not a vector collection",
                        name
                    ),
                }),
            )
                .into_response()
        }
    };

    let point = Point::new(req.id, req.vector, req.payload);

    match collection.stream_insert(point) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(BackpressureError::BufferFull) => {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert("Retry-After", axum::http::HeaderValue::from_static("0.1"));
            (
                StatusCode::TOO_MANY_REQUESTS,
                headers,
                Json(ErrorResponse {
                    error: "Stream buffer full, retry after 100ms".to_string(),
                }),
            )
                .into_response()
        }
        Err(BackpressureError::NotConfigured) => (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "Streaming not configured for this collection".to_string(),
            }),
        )
            .into_response(),
    }
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
    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!(
                        "Collection '{}' not found or is not a vector collection",
                        name
                    ),
                }),
            )
                .into_response()
        }
    };

    let points = collection.get(&[id]);

    match points.into_iter().next().flatten() {
        Some(point) => Json(serde_json::json!({
            "id": point.id,
            "vector": point.vector,
            "payload": point.payload
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Point {} not found", id),
            }),
        )
            .into_response(),
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
#[allow(clippy::unused_async)]
pub async fn delete_point(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, u64)>,
) -> impl IntoResponse {
    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!(
                        "Collection '{}' not found or is not a vector collection",
                        name
                    ),
                }),
            )
                .into_response()
        }
    };

    match collection.delete(&[id]) {
        Ok(()) => Json(serde_json::json!({
            "message": "Point deleted",
            "id": id
        }))
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
