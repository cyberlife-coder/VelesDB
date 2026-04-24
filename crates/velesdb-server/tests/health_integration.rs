//! Integration tests for GET /health and GET /ready endpoints.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use tower::ServiceExt;
use velesdb_core::Database;
use velesdb_server::{AppState, OnboardingMetrics};

// ============================================================================
// GET /health — liveness probe
// ============================================================================

#[tokio::test]
async fn health_returns_200_with_status_ok() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let app = common::create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string(), "version should be present");
}

// ============================================================================
// GET /ready — readiness probe
// ============================================================================

#[tokio::test]
async fn ready_returns_200_when_db_loaded() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let app = common::create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ready");
    assert!(json["version"].is_string(), "version should be present");
}

#[tokio::test]
async fn ready_returns_503_when_not_ready() {
    let temp_dir = tempfile::tempdir().expect("temp dir");

    // Build app with ready=false to simulate startup
    let db = Database::open(temp_dir.path()).expect("open db");
    let state = Arc::new(AppState {
        db,
        onboarding_metrics: OnboardingMetrics::default(),
        query_limits: parking_lot::RwLock::new(velesdb_core::guardrails::QueryLimits::default()),
        ready: std::sync::atomic::AtomicBool::new(false),
        operational_metrics: velesdb_core::metrics::OperationalMetrics::shared(),
        traversal_metrics: std::sync::Arc::new(velesdb_core::metrics::TraversalMetrics::new()),
        query_duration_histogram: std::sync::Arc::new(
            velesdb_core::metrics::DurationHistogram::new(),
        ),
    });

    let app = Router::new()
        .route("/ready", get(velesdb_server::readiness_check))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "not_ready");
    assert!(json["version"].is_string(), "version should be present");
}

// ============================================================================
// Auth bypass — both endpoints must be public
// ============================================================================

#[tokio::test]
async fn health_bypasses_auth() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let app = common::create_test_app_with_auth(&temp_dir, vec!["secret-key".to_string()]);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn ready_bypasses_auth() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let app = common::create_test_app_with_auth(&temp_dir, vec!["secret-key".to_string()]);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should get 200 (not 401) even without auth header
    assert_eq!(response.status(), StatusCode::OK);
}
