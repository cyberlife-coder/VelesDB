//! Integration tests for VelesDB REST API.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

use velesdb_core::Database;
use velesdb_server::{
    batch_search, create_collection, delete_collection, delete_point, get_collection, get_point,
    health_check, list_collections, search, upsert_points, AppState,
};

/// Helper to create test app with all routes
fn create_test_app(temp_dir: &TempDir) -> Router {
    let db = Database::open(temp_dir.path()).expect("Failed to open database");
    let state = Arc::new(AppState { db });

    Router::new()
        .route("/health", get(health_check))
        .route(
            "/collections",
            get(list_collections).post(create_collection),
        )
        .route(
            "/collections/{name}",
            get(get_collection).delete(delete_collection),
        )
        .route("/collections/{name}/points", post(upsert_points))
        .route(
            "/collections/{name}/points/{id}",
            get(get_point).delete(delete_point),
        )
        .route("/collections/{name}/search", post(search))
        .route("/collections/{name}/search/batch", post(batch_search))
        .with_state(state)
}

#[tokio::test]
async fn test_health_check() {
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

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert_eq!(json["status"], "healthy");
}

#[tokio::test]
async fn test_create_collection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "test_collection",
                        "dimension": 128,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_list_collections() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert!(json["collections"].is_array());
}

#[tokio::test]
async fn test_collection_not_found() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/nonexistent")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_invalid_metric() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "test",
                        "dimension": 128,
                        "metric": "invalid_metric"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_upsert_and_search() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create collection via API
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "vectors",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::CREATED);

    // Upsert points
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/vectors/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
                            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
                            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0]}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);

    // Search
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/vectors/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 2
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert!(json["results"].is_array());
}

#[tokio::test]
async fn test_batch_search() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create collection via API
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "vectors",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::CREATED);

    // Upsert points via API
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/vectors/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
                            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);

    // Batch search
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/vectors/search/batch")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "searches": [
                            {"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 1},
                            {"vector": [0.0, 1.0, 0.0, 0.0], "top_k": 1}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert!(json["results"].is_array());
    assert_eq!(json["results"].as_array().expect("Not an array").len(), 2);
    assert!(json["timing_ms"].is_number());
}
