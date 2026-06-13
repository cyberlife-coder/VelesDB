//! Binary wire-format bulk upsert handler (`upsert_points_raw`).
//!
//! Accepts a length-prefixed, tightly-packed binary body for zero-copy bulk
//! insertion of `(id, vector)` pairs, avoiding the per-point JSON overhead of
//! [`super::upsert_points`]. Payloads are not carried on this path — use the
//! JSON endpoint when you need them.
//!
//! # Wire format (`application/octet-stream`, little-endian)
//!
//! All multi-byte integers and `f32`s are little-endian. The body is:
//!
//! ```text
//! offset  size                     field
//! ------  -----------------------  --------------------------------------
//! 0       4                        magic  = b"VRB1"  (Veles Raw Bulk v1)
//! 4       4                        count  : u32      (number of points)
//! 8       4                        dim    : u32      (vector dimension)
//! 12      1                        id_width : u8     (must be 8 → u64)
//! 13      3                        reserved (must be 0) — header is 16 bytes
//! 16      count * 8                ids    : [u64; count]
//! 16+8c   count * dim * 4          vectors: [f32; count * dim] (row-major)
//! ```
//!
//! The total body length must be **exactly**
//! `16 + count * 8 + count * dim * 4` bytes; any mismatch is a `400`.
//!
//! The format is deterministic: a given batch always serialises to the same
//! bytes (see [`encode_raw_bulk`] in the tests), so clients and the server
//! agree byte-for-byte.

use axum::{body::Bytes, extract::Path, extract::State, http::StatusCode, response::IntoResponse};
use std::sync::Arc;

use super::upsert_result_to_response;
use crate::handlers::helpers::{error_response, get_vector_collection_or_404};
use crate::types::ErrorResponse;
use crate::AppState;

/// 4-byte magic prefix identifying the v1 raw-bulk wire format.
const RAW_BULK_MAGIC: &[u8; 4] = b"VRB1";

/// Fixed header length in bytes: magic(4) + count(4) + dim(4) + id_width(1) + reserved(3).
const RAW_BULK_HEADER_LEN: usize = 16;

/// The only supported id width: `u64` ids are 8 bytes each.
const RAW_BULK_ID_WIDTH: u8 = 8;

/// Parsed view over a raw-bulk binary body: owned `ids` plus a flat `vectors`
/// buffer and the declared `dimension`.
struct RawBulkBatch {
    ids: Vec<u64>,
    vectors: Vec<f32>,
    dimension: usize,
}

/// Parse the fixed 16-byte header, returning `(count, dim)`.
///
/// Validates the magic, the id width, and the reserved padding so that a
/// malformed or wrong-version body is rejected before any allocation.
fn parse_raw_bulk_header(body: &[u8]) -> Result<(usize, usize), String> {
    if body.len() < RAW_BULK_HEADER_LEN {
        return Err(format!(
            "body too short: {} bytes (header needs {RAW_BULK_HEADER_LEN})",
            body.len()
        ));
    }
    if &body[0..4] != RAW_BULK_MAGIC {
        return Err("bad magic: expected b\"VRB1\"".to_string());
    }
    let count = u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize;
    let dim = u32::from_le_bytes([body[8], body[9], body[10], body[11]]) as usize;
    if body[12] != RAW_BULK_ID_WIDTH {
        return Err(format!(
            "unsupported id_width {}: only {RAW_BULK_ID_WIDTH} (u64) is supported",
            body[12]
        ));
    }
    if body[13] != 0 || body[14] != 0 || body[15] != 0 {
        return Err("reserved header bytes must be zero".to_string());
    }
    Ok((count, dim))
}

/// Compute the expected total body length for `count` points of `dim` floats.
///
/// Returns `Err` on arithmetic overflow rather than panicking.
fn expected_body_len(count: usize, dim: usize) -> Result<usize, String> {
    let ids_bytes = count
        .checked_mul(8)
        .ok_or_else(|| "overflow computing id section length".to_string())?;
    let vec_elems = count
        .checked_mul(dim)
        .ok_or_else(|| "overflow computing vector element count".to_string())?;
    let vec_bytes = vec_elems
        .checked_mul(4)
        .ok_or_else(|| "overflow computing vector section length".to_string())?;
    RAW_BULK_HEADER_LEN
        .checked_add(ids_bytes)
        .and_then(|h| h.checked_add(vec_bytes))
        .ok_or_else(|| "overflow computing total body length".to_string())
}

/// Decode the tightly-packed id section into a `Vec<u64>`.
///
/// The caller validates the exact body length first, so every 8-byte chunk
/// is in bounds; `chunks_exact` drops any trailing partial chunk safely.
fn decode_ids(body: &[u8], count: usize) -> Vec<u64> {
    let start = RAW_BULK_HEADER_LEN;
    let end = start + count * 8;
    body[start..end]
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
        .collect()
}

/// Decode the tightly-packed vector section into a flat `Vec<f32>`.
fn decode_vectors(body: &[u8], count: usize, dim: usize) -> Vec<f32> {
    let start = RAW_BULK_HEADER_LEN + count * 8;
    let end = start + count * dim * 4;
    body[start..end]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Parse a full raw-bulk body into a [`RawBulkBatch`].
///
/// Validates the header and the exact total length before decoding, so all
/// slice accesses in [`decode_ids`] / [`decode_vectors`] are in bounds.
fn parse_raw_bulk_body(body: &[u8]) -> Result<RawBulkBatch, String> {
    let (count, dim) = parse_raw_bulk_header(body)?;
    let expected = expected_body_len(count, dim)?;
    if body.len() != expected {
        return Err(format!(
            "body length {} != expected {expected} (count={count}, dim={dim})",
            body.len()
        ));
    }
    Ok(RawBulkBatch {
        ids: decode_ids(body, count),
        vectors: decode_vectors(body, count, dim),
        dimension: dim,
    })
}

/// Bulk upsert points via the binary wire format.
///
/// Accepts an `application/octet-stream` body in the v1 raw-bulk format
/// documented at the module level: a 16-byte header (magic, count, dim,
/// id_width) followed by tightly-packed `u64` ids and `f32` vectors. The
/// vectors are forwarded to `Collection::upsert_bulk_from_raw` (zero per-point
/// `Point` allocation). Payloads are not supported on this path.
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

    let batch = match parse_raw_bulk_body(&body) {
        Ok(b) => b,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only encoder mirroring the documented wire format. Kept here so
    /// the round-trip test is self-contained and the format stays pinned.
    fn encode_raw_bulk(ids: &[u64], vectors: &[f32], dim: usize) -> Vec<u8> {
        let count = ids.len();
        let mut buf = Vec::with_capacity(RAW_BULK_HEADER_LEN + count * 8 + count * dim * 4);
        buf.extend_from_slice(RAW_BULK_MAGIC);
        let count_u32 = u32::try_from(count).expect("test: count fits u32");
        let dim_u32 = u32::try_from(dim).expect("test: dim fits u32");
        buf.extend_from_slice(&count_u32.to_le_bytes());
        buf.extend_from_slice(&dim_u32.to_le_bytes());
        buf.push(RAW_BULK_ID_WIDTH);
        buf.extend_from_slice(&[0u8; 3]);
        for id in ids {
            buf.extend_from_slice(&id.to_le_bytes());
        }
        for v in vectors {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf
    }

    #[test]
    fn test_roundtrip_parse() {
        let ids = [1u64, 2, 3];
        let vectors = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6];
        let body = encode_raw_bulk(&ids, &vectors, 2);
        let batch = parse_raw_bulk_body(&body).expect("test: valid body parses");
        assert_eq!(batch.ids, vec![1, 2, 3]);
        assert_eq!(batch.vectors, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        assert_eq!(batch.dimension, 2);
    }

    #[test]
    fn test_deterministic_encoding() {
        let ids = [7u64, 42];
        let vectors = [1.0f32, 2.0, 3.0, 4.0];
        let a = encode_raw_bulk(&ids, &vectors, 2);
        let b = encode_raw_bulk(&ids, &vectors, 2);
        assert_eq!(a, b, "encoding must be deterministic");
        // First bytes are the magic, then count=2 LE, then dim=2 LE, id_width=8.
        assert_eq!(&a[0..4], b"VRB1");
        assert_eq!(&a[4..8], &2u32.to_le_bytes());
        assert_eq!(&a[8..12], &2u32.to_le_bytes());
        assert_eq!(a[12], 8);
        assert_eq!(&a[13..16], &[0, 0, 0]);
    }

    #[test]
    fn test_bad_magic_rejected() {
        let mut body = encode_raw_bulk(&[1], &[0.0, 0.0], 2);
        body[0] = b'X';
        assert!(parse_raw_bulk_body(&body).is_err());
    }

    #[test]
    fn test_length_mismatch_rejected() {
        let mut body = encode_raw_bulk(&[1, 2], &[0.0, 0.0, 0.0, 0.0], 2);
        body.pop(); // truncate one byte
        match parse_raw_bulk_body(&body) {
            Ok(_) => panic!("truncated body must fail"),
            Err(err) => assert!(err.contains("body length"), "got: {err}"),
        }
    }

    #[test]
    fn test_short_body_rejected() {
        let body = vec![0u8; 4];
        assert!(parse_raw_bulk_body(&body).is_err());
    }

    #[test]
    fn test_empty_batch_parses() {
        let body = encode_raw_bulk(&[], &[], 4);
        let batch = parse_raw_bulk_body(&body).expect("test: empty batch parses");
        assert!(batch.ids.is_empty());
        assert!(batch.vectors.is_empty());
        assert_eq!(batch.dimension, 4);
    }
}
