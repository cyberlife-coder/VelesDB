//! Integration tests for `POST /collections/{name}/stream/enable` (STREAM-5).
//!
//! Enabling streaming spawns the background drain task so a subsequent
//! `POST /collections/{name}/stream/insert` is accepted (202) instead of being
//! rejected with `409 Conflict` ("streaming not configured").

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

const COLLECTION: &str = "streaming_enable";
const DIM: usize = 4;

async fn post(app: &axum::Router, uri: &str, body: Value) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("test: build request"),
        )
        .await
        .expect("test: request")
}

async fn create_collection(app: &axum::Router) {
    let resp = post(
        app,
        "/collections",
        json!({"name": COLLECTION, "dimension": DIM, "metric": "cosine"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "test setup: create");
}

#[tokio::test]
async fn test_rest_enable_streaming_success() {
    let temp = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp);
    create_collection(&app).await;

    // Before enabling, stream/insert is rejected with 409 (not configured).
    let before = post(
        &app,
        &format!("/collections/{COLLECTION}/stream/insert"),
        json!({"id": 1, "vector": [0.1, 0.2, 0.3, 0.4]}),
    )
    .await;
    assert_eq!(
        before.status(),
        StatusCode::CONFLICT,
        "stream/insert should be 409 before streaming is enabled"
    );

    // Enable streaming with explicit (small) config.
    let enable = post(
        &app,
        &format!("/collections/{COLLECTION}/stream/enable"),
        json!({"bufferSize": 1024, "batchSize": 32, "flushIntervalMs": 20}),
    )
    .await;
    assert_eq!(
        enable.status(),
        StatusCode::OK,
        "enable_streaming should return 200 OK"
    );

    // After enabling, the same insert is accepted (202).
    let after = post(
        &app,
        &format!("/collections/{COLLECTION}/stream/insert"),
        json!({"id": 1, "vector": [0.1, 0.2, 0.3, 0.4]}),
    )
    .await;
    assert_eq!(
        after.status(),
        StatusCode::ACCEPTED,
        "stream/insert should be 202 once streaming is enabled"
    );
}

#[tokio::test]
async fn test_rest_enable_streaming_defaults_empty_body() {
    let temp = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp);
    create_collection(&app).await;

    // Empty body => all fields fall back to engine defaults.
    let enable = post(
        &app,
        &format!("/collections/{COLLECTION}/stream/enable"),
        json!({}),
    )
    .await;
    assert_eq!(
        enable.status(),
        StatusCode::OK,
        "enable_streaming with empty body should succeed via defaults"
    );
}

#[tokio::test]
async fn test_rest_enable_streaming_collection_not_found() {
    let temp = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp);

    let resp = post(&app, "/collections/does_not_exist/stream/enable", json!({})).await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "enable_streaming on a missing collection should return 404"
    );
}
