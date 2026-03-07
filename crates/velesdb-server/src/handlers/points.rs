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

/// Accumulates statistics over an NDJSON stream upsert operation.
#[derive(Default)]
struct StreamUpsertStats {
    inserted: usize,
    malformed: usize,
    failed_upserts: usize,
    /// Number of HTTP/transport errors encountered while reading the request body.
    ///
    /// A non-zero value means the stream was truncated mid-transfer; the response
    /// `inserted` count is therefore a lower bound on how many points were actually
    /// sent by the client.
    network_errors: u64,
}

fn parse_ndjson_line(
    line: &str,
    batch: &mut Vec<Point>,
    stats: &mut StreamUpsertStats,
    point_id_hint: Option<u64>,
) {
    if line.is_empty() {
        return;
    }

    match serde_json::from_str::<Point>(line) {
        Ok(point) => batch.push(point),
        Err(error) => {
            stats.malformed += 1;
            // N-2: include point ID in the warning when it is available from context.
            if let Some(id) = point_id_hint {
                tracing::warn!(
                    error = %error,
                    point_id = id,
                    "Skipping malformed NDJSON point"
                );
            } else {
                tracing::warn!(error = %error, "Skipping malformed NDJSON point");
            }
        }
    }
}

/// Flushes a batch and — if the collection's delta buffer is active — also
/// pushes the entries into the buffer for immediate searchability.
///
/// This is the correct call-site replacement for `flush_point_batch` in the
/// NDJSON stream handler. It avoids the ownership problem (collection is moved
/// into `spawn_blocking`) by snapshotting delta entries before the move.
async fn flush_point_batch_with_delta(
    collection: &VectorCollection,
    batch: &mut Vec<Point>,
    stats: &mut StreamUpsertStats,
) {
    if batch.is_empty() {
        return;
    }

    let points = std::mem::take(batch);
    let batch_size = points.len();

    // Snapshot (id, vector) pairs for delta before moving `points` into spawn_blocking.
    // Only allocate when delta is active to keep the hot path allocation-free.
    // Uses the public `is_delta_active()` API — does not access `inner` directly.
    #[cfg(feature = "persistence")]
    let delta_entries: Vec<(u64, Vec<f32>)> = if collection.is_delta_active() {
        points.iter().map(|p| (p.id, p.vector.clone())).collect()
    } else {
        Vec::new()
    };

    let coll = collection.clone();
    match tokio::task::spawn_blocking(move || coll.upsert_bulk(&points)).await {
        Ok(Ok(inserted)) => {
            stats.inserted += inserted;

            // C-3: push into the delta buffer after a successful upsert so that
            // search can find these points before HNSW is rebuilt.
            #[cfg(feature = "persistence")]
            if !delta_entries.is_empty() {
                collection.push_to_delta_if_active(&delta_entries);
            }
        }
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
    // Must use spawn_blocking to avoid blocking the async runtime.
    //
    // NOTE: This handler intentionally does NOT push points to the delta buffer.
    // The delta buffer is only used by the streaming ingest path
    // (`stream_upsert_points`) during a background HNSW rebuild. Standard
    // upserts go directly through `upsert_bulk`, which inserts straight into
    // the HNSW index and WAL — no buffering needed.
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
///
/// Accepts a `application/x-ndjson` body. Each line is a JSON-encoded [`Point`].
/// Points are accumulated into micro-batches and flushed via `upsert_bulk`.
///
/// The response body includes a `network_errors` field: a non-zero value means
/// the HTTP body stream was truncated (e.g., client disconnect or proxy error),
/// and the server may have received fewer points than the client sent (M-7).
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
                    // N-2: pre-parse `id` field for the warning log. This is a second
                    // JSON parse of the same line (parse_ndjson_line also parses it),
                    // but only occurs on the error path (malformed NDJSON). On the
                    // happy path the id_hint parse result is unused if the line is
                    // well-formed. A future refactor could extract id from the Point
                    // struct inside parse_ndjson_line to eliminate the double parse.
                    let id_hint = serde_json::from_str::<serde_json::Value>(line.trim())
                        .ok()
                        .and_then(|v| v.get("id").and_then(|id| id.as_u64()));
                    parse_ndjson_line(line.trim(), &mut batch, &mut stats, id_hint);

                    if batch.len() >= STREAM_BATCH_SIZE
                        || (!batch.is_empty() && last_flush.elapsed() >= STREAM_BATCH_MAX_WAIT)
                    {
                        flush_point_batch_with_delta(&collection, &mut batch, &mut stats).await;
                        last_flush = Instant::now();
                    }
                }
            }
            Err(error) => {
                // M-7: count network errors so clients can detect partial delivery.
                stats.network_errors += 1;
                tracing::warn!(error = %error, "Error while reading request body stream");
            }
        }
    }

    if !buffer.is_empty() {
        let line = String::from_utf8_lossy(&buffer);
        let id_hint = serde_json::from_str::<serde_json::Value>(line.trim())
            .ok()
            .and_then(|v| v.get("id").and_then(|id| id.as_u64()));
        parse_ndjson_line(line.trim(), &mut batch, &mut stats, id_hint);
    }

    flush_point_batch_with_delta(&collection, &mut batch, &mut stats).await;

    // notify_upsert is intentionally called once at stream completion (not per batch)
    // to avoid triggering auto-reindex threshold checks mid-stream. The HNSW rebuild
    // is deferred until the entire stream is ingested for efficiency.
    if stats.inserted > 0 {
        state.db.notify_upsert(&name, stats.inserted);
    }

    Json(serde_json::json!({
        "message": "Stream processed",
        "inserted": stats.inserted,
        "malformed": stats.malformed,
        "failed_upserts": stats.failed_upserts,
        "network_errors": stats.network_errors
    }))
    .into_response()
}

/// Stream-insert a single point via the bounded ingestion channel.
///
/// Returns 202 Accepted on success, 429 Too Many Requests when the buffer is
/// full (with `Retry-After: 1` header per RFC 7231), 503 Service Unavailable
/// when the drain task has exited, and 404 when the collection is not found.
///
/// This handler is `async` to satisfy Axum's handler contract; it does not
/// perform any async I/O internally (the channel send is non-blocking).
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
        (status = 429, description = "Streaming buffer full — retry after 1 second", body = ErrorResponse),
        (status = 503, description = "Streaming drain task has exited — collection must be reconfigured", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 409, description = "Streaming not configured", body = ErrorResponse)
    )
)]
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

    // M-3: validate vector dimension before sending into the channel.
    let expected_dim = collection.dimension();
    if req.vector.len() != expected_dim {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Vector dimension mismatch: collection expects {expected_dim}, \
                     got {}",
                    req.vector.len()
                ),
            }),
        )
            .into_response();
    }

    let point = Point::new(req.id, req.vector, req.payload);

    match collection.stream_insert(point) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(BackpressureError::BufferFull) => {
            let mut headers = axum::http::HeaderMap::new();
            // N-1: RFC 7231 §7.1.3 requires Retry-After to be a non-negative integer (seconds).
            headers.insert("Retry-After", axum::http::HeaderValue::from_static("1"));
            (
                StatusCode::TOO_MANY_REQUESTS,
                headers,
                Json(ErrorResponse {
                    error: "Stream buffer full, retry after 1s".to_string(),
                }),
            )
                .into_response()
        }
        Err(BackpressureError::DrainTaskDead) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Streaming drain task has exited; the collection must be reconfigured"
                    .to_string(),
            }),
        )
            .into_response(),
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
