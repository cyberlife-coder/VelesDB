#![allow(clippy::doc_markdown)]
//! Integration tests for `POST /collections/{name}/points/raw`
//! (`upsert_points_raw`).
//!
//! The endpoint accepts the deterministic VRB1 binary format (16-byte header
//! plus packed u64 ids and f32 vectors). These tests pin that a valid batch
//! returns the inserted count and the points become retrievable, that a body
//! whose declared dimension differs from the collection is rejected with 400,
//! and that a malformed (bad-magic or truncated) body is rejected with 400.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

const COLLECTION: &str = "raw_bulk";
const DIM: usize = 4;

/// Encode an `(ids, vectors)` batch into the VRB1 wire format (little-endian).
///
/// Mirrors the contract documented on the server handler so the test fails if
/// either side of the format drifts.
fn encode_raw_bulk(ids: &[u64], vectors: &[f32], dim: usize) -> Vec<u8> {
    let count = ids.len();
    let mut buf = Vec::with_capacity(16 + count * 8 + count * dim * 4);
    buf.extend_from_slice(b"VRB1");
    let count_u32 = u32::try_from(count).expect("test: count fits u32");
    let dim_u32 = u32::try_from(dim).expect("test: dim fits u32");
    buf.extend_from_slice(&count_u32.to_le_bytes());
    buf.extend_from_slice(&dim_u32.to_le_bytes());
    buf.push(8u8); // id_width = u64
    buf.extend_from_slice(&[0u8; 3]); // reserved
    for id in ids {
        buf.extend_from_slice(&id.to_le_bytes());
    }
    for v in vectors {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

async fn post_json(app: &axum::Router, uri: &str, body: Value) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("test: build json request"),
        )
        .await
        .expect("test: json request")
}

async fn post_binary(app: &axum::Router, uri: &str, body: Vec<u8>) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/octet-stream")
                .body(Body::from(body))
                .expect("test: build binary request"),
        )
        .await
        .expect("test: binary request")
}

async fn read_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    serde_json::from_slice(&body).expect("test: valid JSON")
}

async fn create_collection(app: &axum::Router) {
    let resp = post_json(
        app,
        "/collections",
        json!({"name": COLLECTION, "dimension": DIM, "metric": "cosine"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "test setup: create");
}

#[tokio::test]
async fn test_raw_bulk_insert_returns_count() {
    let dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&dir);
    create_collection(&app).await;

    let ids = [10u64, 20, 30];
    let vectors = [
        1.0f32, 0.0, 0.0, 0.0, // id 10
        0.0, 1.0, 0.0, 0.0, // id 20
        0.0, 0.0, 1.0, 0.0, // id 30
    ];
    let body = encode_raw_bulk(&ids, &vectors, DIM);

    let resp = post_binary(&app, &format!("/collections/{COLLECTION}/points/raw"), body).await;
    assert_eq!(resp.status(), StatusCode::OK, "raw insert should succeed");
    let json = read_json(resp).await;
    assert_eq!(json["count"], 3, "inserted count should be 3");

    // The points must be retrievable afterwards.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/collections/{COLLECTION}/points/20"))
                .body(Body::empty())
                .expect("test: build get request"),
        )
        .await
        .expect("test: get request");
    assert_eq!(resp.status(), StatusCode::OK, "point 20 should exist");
}

#[tokio::test]
async fn test_raw_bulk_dimension_mismatch_rejected() {
    let dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&dir);
    create_collection(&app).await;

    // Declared dimension 3 != collection dimension 4.
    let ids = [1u64];
    let vectors = [0.1f32, 0.2, 0.3];
    let body = encode_raw_bulk(&ids, &vectors, 3);

    let resp = post_binary(&app, &format!("/collections/{COLLECTION}/points/raw"), body).await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "dimension mismatch must be 400"
    );
}

#[tokio::test]
async fn test_raw_bulk_bad_magic_rejected() {
    let dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&dir);
    create_collection(&app).await;

    let ids = [1u64];
    let vectors = [0.1f32, 0.2, 0.3, 0.4];
    let mut body = encode_raw_bulk(&ids, &vectors, DIM);
    body[0] = b'X'; // corrupt the magic

    let resp = post_binary(&app, &format!("/collections/{COLLECTION}/points/raw"), body).await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "bad magic must be 400"
    );
}

#[tokio::test]
async fn test_raw_bulk_truncated_body_rejected() {
    let dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&dir);
    create_collection(&app).await;

    let ids = [1u64, 2];
    let vectors = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
    let mut body = encode_raw_bulk(&ids, &vectors, DIM);
    body.pop(); // drop a byte → length mismatch

    let resp = post_binary(&app, &format!("/collections/{COLLECTION}/points/raw"), body).await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "truncated body must be 400"
    );
}
