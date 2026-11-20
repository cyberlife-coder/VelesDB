//! NDJSON streaming upsert and bounded ingestion channel handlers.

use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use futures::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::types::{EnableStreamingRequest, ErrorResponse, StreamInsertRequest};
use crate::AppState;
use velesdb_core::{BackpressureError, Point, VectorCollection};

use super::ndjson_lines::{LineSplitter, NdjsonLineLimit};
use crate::handlers::content_type::{Ndjson, RequireContentType};
use crate::handlers::helpers::{
    auto_core_error_response, error_response, get_vector_collection_or_404,
};

const STREAM_BATCH_SIZE: usize = 100;
const STREAM_BATCH_MAX_WAIT: Duration = Duration::from_millis(100);

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

/// Flushes a batch through `upsert_bulk` on the blocking thread pool.
async fn flush_point_batch(
    collection: &VectorCollection,
    batch: &mut Vec<Point>,
    stats: &mut StreamUpsertStats,
) {
    if batch.is_empty() {
        return;
    }

    let points = std::mem::take(batch);
    let batch_size = points.len();

    let coll = collection.clone();
    match tokio::task::spawn_blocking(move || coll.upsert_bulk(&points)).await {
        Ok(Ok(inserted)) => {
            stats.inserted += inserted;
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

/// Stream upsert points using NDJSON.
///
/// Accepts an `application/x-ndjson` body, and refuses any other
/// `Content-Type` with `415`. Each line is one JSON-encoded point,
/// whose `id` is an unsigned 64-bit integer: a line that is not blank and does
/// not parse as one, a string `id` included, is counted in the response's
/// `malformed` field.
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
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 415, description = "Content-Type is not application/x-ndjson", body = ErrorResponse)
    )
)]
pub async fn stream_upsert_points(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    _content_type: RequireContentType<Ndjson>,
    line_limit: Option<Extension<NdjsonLineLimit>>,
    body: Body,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    // The router always sets the limit; a router assembled without it gets
    // the default body size rather than an unbounded line.
    let max_line_bytes = line_limit.map_or(
        crate::routes::DEFAULT_MAX_BODY_SIZE,
        |Extension(NdjsonLineLimit(max_line_bytes))| max_line_bytes,
    );
    let stats = process_ndjson_stream(&collection, body, max_line_bytes).await;

    Json(serde_json::json!({
        "message": "Stream processed",
        "inserted": stats.inserted,
        "malformed": stats.malformed,
        "failed_upserts": stats.failed_upserts,
        "network_errors": stats.network_errors
    }))
    .into_response()
}

/// Pre-parse the `id` field from a JSON line for diagnostic logging.
fn extract_id_hint(line: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_u64()))
}

/// Read an NDJSON body stream, batching points and flushing periodically.
///
/// A line longer than `max_line_bytes` is dropped and counted as malformed;
/// the stream carries on at the next line.
async fn process_ndjson_stream(
    collection: &VectorCollection,
    body: Body,
    max_line_bytes: usize,
) -> StreamUpsertStats {
    let mut stream = body.into_data_stream();
    let mut lines = LineSplitter::new(max_line_bytes);
    let mut batch = Vec::with_capacity(STREAM_BATCH_SIZE);
    let mut stats = StreamUpsertStats::default();
    let mut last_flush = Instant::now();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                let oversized = lines.push(&chunk, |line| {
                    parse_line_bytes(line, &mut batch, &mut stats);
                });
                record_oversized_lines(oversized, max_line_bytes, &mut stats);
                if should_flush(&batch, last_flush) {
                    flush_point_batch(collection, &mut batch, &mut stats).await;
                    last_flush = Instant::now();
                }
            }
            Err(error) => {
                stats.network_errors += 1;
                tracing::warn!(error = %error, "Error while reading request body stream");
            }
        }
    }

    // A final line sent without a trailing newline.
    parse_line_bytes(&lines.finish(), &mut batch, &mut stats);

    flush_point_batch(collection, &mut batch, &mut stats).await;
    stats
}

/// Parses one complete NDJSON line, given as the bytes between two newlines.
fn parse_line_bytes(line_bytes: &[u8], batch: &mut Vec<Point>, stats: &mut StreamUpsertStats) {
    let line = String::from_utf8_lossy(line_bytes);
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let id_hint = extract_id_hint(line);
    parse_ndjson_line(line, batch, stats, id_hint);
}

/// Counts lines dropped for exceeding the line ceiling as malformed.
fn record_oversized_lines(oversized: usize, max_line_bytes: usize, stats: &mut StreamUpsertStats) {
    if oversized > 0 {
        stats.malformed += oversized;
        tracing::warn!(
            oversized,
            max_line_bytes,
            "Skipping NDJSON line longer than the line limit"
        );
    }
}

/// Check whether the current batch should be flushed.
fn should_flush(batch: &[Point], last_flush: Instant) -> bool {
    batch.len() >= STREAM_BATCH_SIZE
        || (!batch.is_empty() && last_flush.elapsed() >= STREAM_BATCH_MAX_WAIT)
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
        (status = 400, description = "A point the collection refuses: wrong dimension (VELES-004), a non-finite component (VELES-005) or a vector on a metadata-only collection (VELES-014); nothing is queued", body = ErrorResponse),
        (status = 429, description = "Streaming buffer full — retry after 1 second", body = ErrorResponse),
        (status = 503, description = "Streaming drain task has exited — collection must be reconfigured", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 409, description = "Streaming not configured", body = ErrorResponse),
        (status = 422, response = crate::types::MalformedBody)
    )
)]
pub async fn stream_insert(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<StreamInsertRequest>,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let point = Point::new(req.id, req.vector, req.payload);
    // Core checks a streamed point as an upsert: its own code and status.
    if let Err(err) = collection.validate_points_for_upsert(std::slice::from_ref(&point)) {
        return auto_core_error_response(&err);
    }
    stream_insert_result_to_response(collection.stream_insert(point))
}

/// Convert a `stream_insert` result into an HTTP response.
fn stream_insert_result_to_response(
    result: Result<(), BackpressureError>,
) -> axum::response::Response {
    match result {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(BackpressureError::BufferFull) => {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert("Retry-After", axum::http::HeaderValue::from_static("1"));
            (
                StatusCode::TOO_MANY_REQUESTS,
                headers,
                Json(ErrorResponse {
                    error: "Stream buffer full, retry after 1s".to_string(),
                    code: None,
                }),
            )
                .into_response()
        }
        Err(BackpressureError::DrainTaskDead) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Streaming drain task has exited; the collection must be reconfigured".to_string(),
        ),
        Err(BackpressureError::NotConfigured) => error_response(
            StatusCode::CONFLICT,
            "Streaming not configured for this collection".to_string(),
        ),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unexpected streaming error: {e}"),
        ),
    }
}

/// Enable streaming ingestion on a collection.
///
/// Spawns the background drain task so subsequent calls to
/// `/collections/{name}/stream/insert` accept points instead of returning a
/// `409 Conflict` "streaming not configured" error. Omitted body fields fall
/// back to the engine defaults. Calling this again replaces the existing
/// ingester (the old drain task is aborted).
///
/// Returns `200 OK` on success, `400 Bad Request` (`VELES-009`) for a config
/// the pipeline cannot run with, and `404 Not Found` when the collection does
/// not exist.
#[utoipa::path(
    post,
    path = "/collections/{name}/stream/enable",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = EnableStreamingRequest,
    responses(
        (status = 200, description = "Streaming enabled", body = Object),
        (status = 400, description = "A buffer_size of 0 or past the channel's limit, or a flush_interval_ms of 0 (VELES-009)", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 422, response = crate::types::MalformedBody)
    )
)]
pub async fn enable_streaming(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<EnableStreamingRequest>,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(e) = enable_streaming_on(&collection, &req) {
        return auto_core_error_response(&e);
    }

    Json(serde_json::json!({
        "message": "Streaming enabled",
        "collection": name,
    }))
    .into_response()
}

/// Enables streaming on the collection when persistence is compiled in.
#[cfg(feature = "persistence")]
fn enable_streaming_on(
    collection: &VectorCollection,
    req: &EnableStreamingRequest,
) -> velesdb_core::Result<()> {
    collection.enable_streaming(req.to_config())
}

/// No-op when persistence is disabled (the streaming pipeline is unavailable).
#[cfg(not(feature = "persistence"))]
#[allow(clippy::unnecessary_wraps)] // Reason: same signature as the persistence build.
fn enable_streaming_on(
    _collection: &VectorCollection,
    _req: &EnableStreamingRequest,
) -> velesdb_core::Result<()> {
    Ok(())
}
