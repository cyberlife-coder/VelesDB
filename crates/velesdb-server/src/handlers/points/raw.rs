//! Binary wire-format bulk upsert handler (`upsert_points_raw`).
//!
//! Accepts a length-prefixed, tightly-packed binary body for zero-copy bulk
//! insertion of `(id, vector)` pairs, avoiding the per-point JSON overhead of
//! [`super::upsert_points`]. Payloads are not carried on this path — use the
//! JSON endpoint when you need them.
//!
//! The wire format (VRB1) and its codec live in the shared
//! [`velesdb_core::wire::vrb1`] module, so the server and the CLI agree
//! byte-for-byte without duplicating the parser.

use axum::{body::Bytes, extract::Path, extract::State, http::StatusCode, response::IntoResponse};
use std::sync::Arc;
use velesdb_core::wire::vrb1;

use super::upsert_result_to_response;
use crate::handlers::helpers::{error_response, get_vector_collection_or_404};
use crate::types::ErrorResponse;
use crate::AppState;

/// Bulk upsert points via the binary wire format.
///
/// Accepts an `application/octet-stream` body in the VRB1 raw-bulk format
/// (a 16-byte header — magic, count, dim, id_width — followed by tightly-packed
/// `u64` ids and `f32` vectors). The vectors are forwarded to
/// `Collection::upsert_bulk_from_raw` (zero per-point `Point` allocation).
/// Payloads are not supported on this path.
#[utoipa::path(
    post,
    path = "/collections/{name}/points/raw",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body(content = String, content_type = "application/octet-stream", description = "VRB1 binary bulk format: header + packed u64 ids + packed f32 vectors"),
    responses(
        (status = 200, description = "Points upserted", body = Object),
        (status = 400, description = "Malformed body or dimension mismatch", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn upsert_points_raw(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let batch = match vrb1::decode(&body) {
        Ok(b) => b,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    let dimension = batch.dimension;
    let ids = batch.ids;
    let vectors = batch.vectors;

    // upsert_bulk_from_raw is blocking (HNSW insertion + I/O) — spawn_blocking
    // keeps the async runtime free.
    let result = tokio::task::spawn_blocking(move || {
        collection.upsert_bulk_from_raw(&vectors, &ids, dimension, None)
    })
    .await;

    upsert_result_to_response(&state, &name, result)
}
