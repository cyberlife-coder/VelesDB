#![allow(clippy::doc_markdown)]
//! Integration tests for `VelesDB` REST API.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

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
    // Results should contain docs matching "rust" (ids 1 and 3)
    let ids: Vec<i64> = results.iter().filter_map(|r| r["id"].as_i64()).collect();
    assert!(
        ids.contains(&1) || ids.contains(&3),
        "Should find rust-related docs"
    );
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
                            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"title": "Rust performance", "content": "Rust is fast"}}
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

// =============================================================================
// VelesQL Advanced E2E Tests (EPIC-011/US-002)
// =============================================================================

#[tokio::test]
async fn test_velesql_order_by_similarity() {
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
                        "name": "similarity_test",
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
                .uri("/collections/similarity_test/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"name": "exact"}},
                            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"name": "close"}},
                            {"id": 3, "vector": [0.5, 0.5, 0.0, 0.0], "payload": {"name": "medium"}},
                            {"id": 4, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"name": "far"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // Query with ORDER BY similarity()
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM similarity_test WHERE vector NEAR $v ORDER BY similarity(vector, $v) DESC LIMIT 10",
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
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    let results = json["results"].as_array().expect("Not an array");
    assert!(!results.is_empty());
    // First result should be the exact match (id=1)
    assert_eq!(results[0]["id"], 1);
}

#[tokio::test]
async fn test_velesql_where_filter() {
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
                        "name": "filter_test",
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

    // Upsert points with various categories
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/filter_test/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"category": "tech", "price": 100}},
                            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"category": "tech", "price": 200}},
                            {"id": 3, "vector": [0.8, 0.2, 0.0, 0.0], "payload": {"category": "science", "price": 150}},
                            {"id": 4, "vector": [0.7, 0.3, 0.0, 0.0], "payload": {"category": "tech", "price": 50}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // Query with WHERE filter on category
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM filter_test WHERE vector NEAR $v AND category = 'tech' LIMIT 10",
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
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    let results = json["results"].as_array().expect("Not an array");
    // Should only return tech category items (ids 1, 2, 4)
    assert_eq!(results.len(), 3);
    for r in results {
        assert_eq!(r["payload"]["category"], "tech");
    }
}

#[tokio::test]
async fn test_velesql_limit_offset() {
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
                        "name": "pagination_test",
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

    // Upsert multiple points
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/pagination_test/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
                            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0]},
                            {"id": 3, "vector": [0.8, 0.2, 0.0, 0.0]},
                            {"id": 4, "vector": [0.7, 0.3, 0.0, 0.0]},
                            {"id": 5, "vector": [0.6, 0.4, 0.0, 0.0]}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // Query with LIMIT 2 (basic pagination)
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM pagination_test WHERE vector NEAR $v LIMIT 2",
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
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    let results = json["results"].as_array().expect("Not an array");
    assert_eq!(results.len(), 2); // LIMIT 2 should return exactly 2 results
                                  // First result should be most similar (id=1)
    assert_eq!(results[0]["id"], 1);
}

#[tokio::test]
async fn test_velesql_select_specific_columns() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create and populate collection
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "columns_test",
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

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/columns_test/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"name": "doc1", "author": "alice", "year": 2024}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // Query selecting specific columns
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT id, name, year FROM columns_test WHERE vector NEAR $v LIMIT 1",
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
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    let results = json["results"].as_array().expect("Not an array");
    assert_eq!(results.len(), 1);
    // Should have requested fields
    assert_eq!(results[0]["id"], 1);
}

#[tokio::test]
async fn test_velesql_case_insensitive_keywords() {
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
                        "name": "case_test",
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

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/case_test/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // Query with mixed case keywords (SQL standard)
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "select * from case_test where vector near $v limit 10",
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
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert!(json["results"].is_array());
    assert_eq!(json["results"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_velesql_collection_not_found() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM nonexistent WHERE vector NEAR $v LIMIT 10",
                        "params": {"v": [1.0, 0.0, 0.0, 0.0]}
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    // Should return NOT_FOUND for missing collection
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// =============================================================================
// Graph E2E Tests (EPIC-011/US-001)
// Graph handlers now delegate to Collection — collection must exist first.
// =============================================================================

/// Helper: create a collection via HTTP (needed before graph operations).
async fn create_graph_collection(app: &axum::Router, name: &str) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": name,
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
}

#[tokio::test]
async fn test_graph_add_edge() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Collection must exist before adding edges
    create_graph_collection(&app, "test").await;

    // Add edge
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/test/graph/edges")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "id": 1,
                        "source": 100,
                        "target": 200,
                        "label": "KNOWS",
                        "properties": {"weight": 0.5}
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
async fn test_graph_get_edges_by_label() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    create_graph_collection(&app, "test").await;

    // Add edges
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/test/graph/edges")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "id": 1,
                        "source": 100,
                        "target": 200,
                        "label": "KNOWS"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/test/graph/edges")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "id": 2,
                        "source": 200,
                        "target": 300,
                        "label": "FOLLOWS"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    // Get edges by label
    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/test/graph/edges?label=KNOWS")
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

    assert_eq!(json["count"], 1);
    assert_eq!(json["edges"][0]["label"], "KNOWS");
    assert_eq!(json["edges"][0]["source"], 100);
    assert_eq!(json["edges"][0]["target"], 200);
}

#[tokio::test]
async fn test_graph_get_edges_missing_label() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Get edges without label should fail
    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/test/graph/edges")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_graph_traverse_bfs() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    create_graph_collection(&app, "graph_test").await;

    // Build a graph: 1 -> 2 -> 3 -> 4
    for (id, src, tgt) in [(1, 1, 2), (2, 2, 3), (3, 3, 4)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/collections/graph_test/graph/edges")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "id": id,
                            "source": src,
                            "target": tgt,
                            "label": "KNOWS"
                        })
                        .to_string(),
                    ))
                    .expect("Failed to build request"),
            )
            .await
            .expect("Request failed");
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // Traverse from node 1
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/graph_test/graph/traverse")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "source": 1,
                        "strategy": "bfs",
                        "max_depth": 3,
                        "limit": 100
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
    assert_eq!(results.len(), 3); // Should find nodes 2, 3, 4

    // Check stats
    assert_eq!(json["stats"]["visited"], 3);
    assert_eq!(json["stats"]["depth_reached"], 3);
}

#[tokio::test]
async fn test_graph_traverse_dfs() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    create_graph_collection(&app, "dfs_test").await;

    // Build graph
    for (id, src, tgt) in [(1, 1, 2), (2, 2, 3)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/collections/dfs_test/graph/edges")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "id": id,
                            "source": src,
                            "target": tgt,
                            "label": "LINKS"
                        })
                        .to_string(),
                    ))
                    .expect("Failed to build request"),
            )
            .await
            .expect("Request failed");
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // DFS traverse
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/dfs_test/graph/traverse")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "source": 1,
                        "strategy": "dfs",
                        "max_depth": 5,
                        "limit": 10
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
    assert_eq!(json["results"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_graph_traverse_with_rel_type_filter() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    create_graph_collection(&app, "filter_test").await;

    // Build graph with mixed edge types: 1 -KNOWS-> 2 -WROTE-> 3
    let edges = [(1, 1, 2, "KNOWS"), (2, 2, 3, "WROTE")];
    for (id, src, tgt, label) in edges {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/collections/filter_test/graph/edges")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "id": id,
                            "source": src,
                            "target": tgt,
                            "label": label
                        })
                        .to_string(),
                    ))
                    .expect("Failed to build request"),
            )
            .await
            .expect("Request failed");
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // Traverse with KNOWS filter only
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/filter_test/graph/traverse")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "source": 1,
                        "strategy": "bfs",
                        "max_depth": 5,
                        "limit": 100,
                        "rel_types": ["KNOWS"]
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

    // Should only find node 2 (KNOWS), not node 3 (WROTE)
    let results = json["results"].as_array().expect("Not an array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["target_id"], 2);
}

#[tokio::test]
async fn test_graph_traverse_invalid_strategy() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/test/graph/traverse")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "source": 1,
                        "strategy": "invalid",
                        "max_depth": 3
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
async fn test_graph_node_degree() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    create_graph_collection(&app, "degree_test").await;

    // Build graph: 1 -> 2, 3 -> 2, 2 -> 4
    // Node 2 has in_degree=2, out_degree=1
    let edges = [(1, 1, 2, "KNOWS"), (2, 3, 2, "KNOWS"), (3, 2, 4, "KNOWS")];
    for (id, src, tgt, label) in edges {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/collections/degree_test/graph/edges")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "id": id,
                            "source": src,
                            "target": tgt,
                            "label": label
                        })
                        .to_string(),
                    ))
                    .expect("Failed to build request"),
            )
            .await
            .expect("Request failed");
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // Get degree of node 2
    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/degree_test/graph/nodes/2/degree")
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

    assert_eq!(json["in_degree"], 2);
    assert_eq!(json["out_degree"], 1);
}
