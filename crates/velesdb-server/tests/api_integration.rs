#![allow(clippy::doc_markdown)]
//! Integration tests for `VelesDB` REST API.

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
    health_check, hybrid_search, list_collections, query, search, text_search, upsert_points,
    AppState,
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
        .route("/collections/{name}/search/text", post(text_search))
        .route("/collections/{name}/search/hybrid", post(hybrid_search))
        .route("/query", post(query))
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

#[tokio::test]
async fn test_velesql_query() {
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
                        "name": "docs",
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

    // Upsert points with payloads
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/docs/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"category": "tech", "price": 100}},
                            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"category": "science", "price": 50}},
                            {"id": 3, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"category": "tech", "price": 200}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);

    // Execute VelesQL query
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
                        "params": {
                            "v": [1.0, 0.0, 0.0, 0.0]
                        }
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
    assert!(json["timing_ms"].is_number());
    assert!(json["rows_returned"].is_number());
}

#[tokio::test]
async fn test_velesql_query_syntax_error() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Execute invalid VelesQL query
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELEC * FROM docs",
                        "params": {}
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// =============================================================================
// BM25 Text Search Tests
// =============================================================================

#[tokio::test]
async fn test_text_search() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create collection
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "docs",
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

    // Upsert points with text payloads
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/docs/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"content": "Rust programming language"}},
                            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"content": "Python is great"}},
                            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"content": "Rust is fast"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // Text search for "rust"
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/docs/search/text")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "rust",
                        "top_k": 10
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
    let results = json["results"].as_array().expect("Not an array");
    assert_eq!(results.len(), 2); // Should find docs 1 and 3
}

#[tokio::test]
async fn test_hybrid_search() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create collection
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "docs",
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
                .uri("/collections/docs/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"content": "Rust programming"}},
                            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"content": "Python programming"}},
                            {"id": 3, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"content": "Rust performance"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // Hybrid search: vector similar to [1,0,0,0] AND text "rust"
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/docs/search/hybrid")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "query": "rust",
                        "top_k": 10,
                        "vector_weight": 0.5
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
    let results = json["results"].as_array().expect("Not an array");
    assert!(!results.is_empty());
    // Doc 1 should rank high (matches both vector and text)
    assert_eq!(results[0]["id"], 1);
}

#[tokio::test]
async fn test_text_search_collection_not_found() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/nonexistent/search/text")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "test",
                        "top_k": 10
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// =============================================================================
// VelesQL MATCH clause tests
// =============================================================================

#[tokio::test]
async fn test_velesql_match_only() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create collection
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "articles",
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

    // Upsert points with text
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/articles/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Rust programming", "content": "Learn Rust"}},
                            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "Python tutorial", "content": "Learn Python"}},
                            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"title": "Rust performance", "content": "Fast code"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // VelesQL query with MATCH only
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM articles WHERE content MATCH 'rust' LIMIT 10",
                        "params": {}
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
    let results = json["results"].as_array().expect("Not an array");
    assert_eq!(results.len(), 2); // Docs 1 and 3 contain "rust"
}

#[tokio::test]
async fn test_velesql_hybrid_near_and_match() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create collection
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "docs",
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
                .uri("/collections/docs/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"content": "Rust programming"}},
                            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"content": "Python programming"}},
                            {"id": 3, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"content": "Rust performance"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // VelesQL with NEAR + MATCH (hybrid)
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM docs WHERE vector NEAR $v AND content MATCH 'rust' LIMIT 10",
                        "params": {"v": [1.0, 0.0, 0.0, 0.0]}
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
        .expect("Request failed");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert!(json["results"].is_array());
    let results = json["results"].as_array().expect("Not an array");
    assert!(!results.is_empty());
    // Doc 1 should rank highest (matches both vector and text)
    assert_eq!(results[0]["id"], 1);
}

// =============================================================================
// Storage Mode Tests (SQ8, Binary quantization)
// =============================================================================

#[tokio::test]
async fn test_create_collection_with_sq8_storage() {
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
                        "name": "sq8_vectors",
                        "dimension": 128,
                        "metric": "cosine",
                        "storage_mode": "sq8"
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
async fn test_create_collection_with_binary_storage() {
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
                        "name": "binary_vectors",
                        "dimension": 128,
                        "metric": "cosine",
                        "storage_mode": "binary"
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
async fn test_create_collection_invalid_storage_mode() {
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
                        "name": "invalid_storage",
                        "dimension": 128,
                        "metric": "cosine",
                        "storage_mode": "invalid_mode"
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
async fn test_sq8_collection_upsert_and_search() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create SQ8 collection
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "sq8_test",
                        "dimension": 4,
                        "metric": "cosine",
                        "storage_mode": "sq8"
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
                .uri("/collections/sq8_test/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
                            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
                            {"id": 3, "vector": [0.9, 0.1, 0.0, 0.0]}
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
                .uri("/collections/sq8_test/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 3
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
    let results = json["results"].as_array().expect("Not an array");
    assert_eq!(results.len(), 3);
    // First result should be exact match
    assert_eq!(results[0]["id"], 1);
}
