//! Authentication middleware integration tests.

mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::get,
    Router,
};
use common::create_test_app;
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;
use velesdb_core::Database;
use velesdb_server::{auth_middleware, health_check, list_collections, AppState};

/// Helper: create test app WITH auth middleware and a configured API key.
fn create_auth_app(temp_dir: &TempDir, api_key: Option<&str>) -> Router {
    let db = Database::open(temp_dir.path()).expect("Failed to open database");
    let state = Arc::new(AppState {
        db,
        api_key: api_key.map(String::from),
    });

    Router::new()
        .route("/health", get(health_check))
        .route("/collections", get(list_collections))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth_middleware))
}

#[tokio::test]
async fn test_health_no_auth_required() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_auth_app(&temp_dir, Some("test-secret-key"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    // /health is exempt from auth
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_unauthenticated_request_returns_401() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_auth_app(&temp_dir, Some("test-secret-key"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    assert!(json["error"].as_str().unwrap().contains("Authorization"));
}

#[tokio::test]
async fn test_wrong_api_key_returns_401() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_auth_app(&temp_dir, Some("correct-key"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections")
                .header(header::AUTHORIZATION, "Bearer wrong-key")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    assert!(json["error"].as_str().unwrap().contains("Invalid API key"));
}

#[tokio::test]
async fn test_correct_api_key_returns_200() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_auth_app(&temp_dir, Some("my-secret"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections")
                .header(header::AUTHORIZATION, "Bearer my-secret")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_no_auth_in_dev_mode() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    // No API key = dev mode
    let app = create_auth_app(&temp_dir, None);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    // Dev mode: no auth required
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_malformed_auth_header_returns_401() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_auth_app(&temp_dir, Some("test-key"));

    // Basic auth instead of Bearer
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/collections")
                .header(header::AUTHORIZATION, "Basic dXNlcjpwYXNz")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Empty Bearer token
    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections")
                .header(header::AUTHORIZATION, "Bearer ")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_existing_tests_still_pass_without_auth() {
    // Existing tests use create_test_app which sets api_key: None (dev mode)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);
}
