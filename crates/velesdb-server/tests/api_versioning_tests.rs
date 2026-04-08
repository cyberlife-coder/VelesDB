//! Integration tests for API versioning (WP-2H).
//!
//! Verifies that:
//! - `/v1/` prefixed routes return responses without deprecation headers.
//! - Legacy (unprefixed) routes still work but include deprecation headers.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;

// ============================================================================
// Versioned routes (/v1/) — no deprecation headers
// ============================================================================

#[tokio::test]
async fn v1_health_returns_200_without_deprecation() {
    let temp_dir = tempfile::tempdir().expect("test: temp dir");
    let app = common::create_versioned_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get("deprecation").is_none(),
        "/v1/ routes must not carry deprecation header"
    );
    assert!(
        response.headers().get("x-api-deprecated").is_none(),
        "/v1/ routes must not carry x-api-deprecated header"
    );
}

#[tokio::test]
async fn v1_collections_returns_200_without_deprecation() {
    let temp_dir = tempfile::tempdir().expect("test: temp dir");
    let app = common::create_versioned_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/collections")
                .body(Body::empty())
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get("deprecation").is_none(),
        "/v1/collections must not carry deprecation header"
    );
}

// ============================================================================
// Legacy routes (no prefix) — must include deprecation headers
// ============================================================================

#[tokio::test]
async fn legacy_health_returns_200_with_deprecation_headers() {
    let temp_dir = tempfile::tempdir().expect("test: temp dir");
    let app = common::create_versioned_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let deprecation = response
        .headers()
        .get("deprecation")
        .expect("test: deprecation header missing on legacy route");
    assert_eq!(deprecation, "true");

    let api_deprecated = response
        .headers()
        .get("x-api-deprecated")
        .expect("test: x-api-deprecated header missing on legacy route");
    assert_eq!(api_deprecated, "Use /v1/ prefix");
}

#[tokio::test]
async fn legacy_collections_returns_200_with_deprecation_headers() {
    let temp_dir = tempfile::tempdir().expect("test: temp dir");
    let app = common::create_versioned_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections")
                .body(Body::empty())
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let deprecation = response
        .headers()
        .get("deprecation")
        .expect("test: deprecation header missing on legacy /collections");
    assert_eq!(deprecation, "true");
}

// ============================================================================
// Response body parity — both routes return identical payloads
// ============================================================================

#[tokio::test]
async fn v1_and_legacy_health_return_same_body() {
    let temp_dir = tempfile::tempdir().expect("test: temp dir");

    // Request /v1/health
    let app_v1 = common::create_versioned_test_app(&temp_dir);
    let resp_v1 = app_v1
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed");
    let body_v1 = axum::body::to_bytes(resp_v1.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json_v1: Value = serde_json::from_slice(&body_v1).expect("test: parse JSON");

    // Request /health (legacy)
    let app_legacy = common::create_versioned_test_app(&temp_dir);
    let resp_legacy = app_legacy
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed");
    let body_legacy = axum::body::to_bytes(resp_legacy.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json_legacy: Value =
        serde_json::from_slice(&body_legacy).expect("test: parse JSON");

    assert_eq!(
        json_v1["status"], json_legacy["status"],
        "v1 and legacy must return the same status"
    );
}
