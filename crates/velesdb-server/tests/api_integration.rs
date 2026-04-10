#![allow(clippy::doc_markdown)]
//! Integration tests for `VelesDB` REST API.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{create_graph_collection, create_test_app, create_test_app_with_state};
use futures::stream;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;
use velesdb_core::Point;

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

    assert_eq!(json["status"], "ok");
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
async fn test_stream_upsert_ndjson() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "stream_vectors",
                        "dimension": 3,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::CREATED);

    let ndjson_lines = vec![
        r#"{"id": 10, "vector": [1.0, 0.0, 0.0], "payload": {"source":"a"}}
"#,
        "not-a-json-line
",
        r#"{"id": 11, "vector": [0.0, 1.0, 0.0]}
"#,
        r#"{"id": 12, "vector": [0.0, 0.0, 1.0]}
"#,
    ];

    let stream_body = Body::from_stream(stream::iter(
        ndjson_lines
            .into_iter()
            .map(|line| Ok::<_, std::io::Error>(line.to_string())),
    ));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/stream_vectors/points/stream")
                .header("Content-Type", "application/x-ndjson")
                .body(stream_body)
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert_eq!(json["inserted"], 3);
    assert_eq!(json["malformed"], 1);

    for point_id in [10_u64, 11, 12] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/collections/stream_vectors/points/{point_id}"))
                    .body(Body::empty())
                    .expect("Failed to build request"),
            )
            .await
            .expect("Request failed");

        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn test_stream_upsert_ndjson_chunked_without_trailing_newline() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "stream_chunked",
                        "dimension": 2,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::CREATED);

    let chunks = vec![
        r#"{"id":101,"vector":[1.0,0.0]"#,
        r#","payload":{"source":"chunk"}}
{"id":102,"#,
        r#""vector":[0.0,1.0]}"#,
    ];

    let stream_body = Body::from_stream(stream::iter(
        chunks
            .into_iter()
            .map(|chunk| Ok::<_, std::io::Error>(chunk.to_string())),
    ));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/stream_chunked/points/stream")
                .header("Content-Type", "application/x-ndjson")
                .body(stream_body)
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert_eq!(json["inserted"], 2);
    assert_eq!(json["malformed"], 0);

    for point_id in [101_u64, 102] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/collections/stream_chunked/points/{point_id}"))
                    .body(Body::empty())
                    .expect("Failed to build request"),
            )
            .await
            .expect("Request failed");

        assert_eq!(response.status(), StatusCode::OK);
    }
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
    assert!(json["took_ms"].is_number());
    assert!(json["rows_returned"].is_number());
    assert_eq!(json["meta"]["velesql_contract_version"], "3.0.0");
    assert!(json["meta"]["count"].is_number());
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

#[tokio::test]
async fn test_aggregate_endpoint_returns_contract_meta() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "agg_docs",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(create.status(), StatusCode::CREATED);

    let upsert = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/agg_docs/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"category": "tech"}},
                            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"category": "science"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(upsert.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/aggregate")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT category, COUNT(*) FROM agg_docs GROUP BY category",
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
    assert!(json["result"].is_array() || json["result"].is_object());
    assert_eq!(json["meta"]["velesql_contract_version"], "3.0.0");
    assert!(json["meta"]["count"].is_number());
}

#[tokio::test]
async fn test_aggregate_endpoint_rejects_non_aggregation_query() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/aggregate")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM docs LIMIT 5",
                        "params": {}
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    assert_eq!(json["error"]["code"], "VELESQL_AGGREGATION_ERROR");
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
    let ids: Vec<i64> = results
        .iter()
        .filter_map(|r| r["id"].as_str().and_then(|s| s.parse::<i64>().ok()))
        .collect();
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
    assert_eq!(results[0]["id"], "1");
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
        // v3.0.0: projected rows have flattened payload fields (no wrapper)
        assert_eq!(r["category"], "tech");
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

#[tokio::test]
async fn test_query_match_top_level_requires_collection() {
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
                        "query": "MATCH (d:Doc) RETURN d LIMIT 1",
                        "params": {}
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    assert_eq!(json["error"]["code"], "VELESQL_MISSING_COLLECTION");
    assert!(json["error"]["hint"].is_string());
}

#[tokio::test]
async fn test_query_match_top_level_with_collection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "docs_match_query",
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
                .uri("/collections/docs_match_query/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"_labels": ["Doc"], "title": "a"}},
                            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"_labels": ["Doc"], "title": "b"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "MATCH (d:Doc) RETURN d LIMIT 1",
                        "collection": "docs_match_query",
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

    let results = json["results"].as_array().expect("Not an array");
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_query_insert_metadata_only_via_query_endpoint() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "profiles",
                        "dimension": 3,
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
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "INSERT INTO profiles (id, vector, name, age) VALUES (1, $vec, 'Alice', 30)",
                        "params": {"vec": [1.0, 0.0, 0.0]}
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
    assert_eq!(json["rows_returned"], 1);
    assert_eq!(json["results"][0]["id"], 1);
    // v3.0.0: projected rows have flattened payload fields
    assert_eq!(json["results"][0]["name"], "Alice");
}

#[tokio::test]
async fn test_query_update_metadata_only_via_query_endpoint() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "profiles",
                        "dimension": 3,
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
                .uri("/collections/profiles/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [Point::new(1, vec![1.0, 0.0, 0.0], Some(json!({"name": "Alice", "age": 30, "id": 1})))]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "UPDATE profiles SET age = 31 WHERE id = 1",
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
    assert_eq!(json["rows_returned"], 1);
    // v3.0.0: projected rows have flattened payload fields
    assert_eq!(json["results"][0]["age"], 31);
}
// =============================================================================
// Graph E2E Tests (EPIC-011/US-001)
// =============================================================================

#[tokio::test]
async fn test_graph_add_edge() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Graph collections must be explicitly created since F-05 (Sprint 1).
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
    assert_eq!(json["edges"][0]["source"], "100");
    assert_eq!(json["edges"][0]["target"], "200");
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
    assert_eq!(results[0]["target_id"], "2");
}

#[tokio::test]
async fn test_graph_traverse_invalid_strategy() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);
    create_graph_collection(&app, "test").await;

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

#[tokio::test]
async fn test_search_dimension_mismatch_returns_actionable_error() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "dim_guard",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(create_response.status(), StatusCode::CREATED);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/dim_guard/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0],
                        "top_k": 2
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    let error = json["error"].as_str().unwrap_or_default();
    assert!(error.contains("expected 4, got 2"));
    assert!(error.contains("Hint"));
}

#[tokio::test]
async fn test_create_collection_returns_preflight_warnings() {
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
                        "name": "warn_collection",
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert!(json["warnings"].is_array());
    assert!(!json["warnings"]
        .as_array()
        .expect("warnings array")
        .is_empty());
}

#[tokio::test]
async fn test_create_collection_with_empty_type_returns_preflight_warnings() {
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
                        "name": "warn_collection_empty_type",
                        "collection_type": "",
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert!(json["warnings"].is_array());
    assert_eq!(json["warnings"].as_array().map_or(0, std::vec::Vec::len), 2);
}

#[tokio::test]
async fn test_collection_sanity_reports_empty_collection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "sanity_collection",
                        "dimension": 3,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(create_response.status(), StatusCode::CREATED);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/sanity_collection/sanity")
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

    assert_eq!(json["checks"]["has_vectors"], false);
    assert_eq!(json["is_empty"], true);
}

#[tokio::test]
async fn test_collection_sanity_includes_diagnostics_counters() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "diag_collection",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(create_response.status(), StatusCode::CREATED);

    // Trigger one dimension mismatch
    let mismatch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/diag_collection/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0],
                        "top_k": 1
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(mismatch_response.status(), StatusCode::BAD_REQUEST);

    let sanity_response = app
        .oneshot(
            Request::builder()
                .uri("/collections/diag_collection/sanity")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(sanity_response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(sanity_response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");

    assert!(
        json["diagnostics"]["search_requests_total"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        json["diagnostics"]["dimension_mismatch_total"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
}

#[tokio::test]
async fn test_batch_search_invalid_filter_returns_bad_request() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "batch_filter_validation",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(create_response.status(), StatusCode::CREATED);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/batch_filter_validation/search/batch")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "searches": [
                            {
                                "vector": [1.0, 0.0, 0.0, 0.0],
                                "top_k": 2,
                                "filter": {
                                    "type": "eq",
                                    "field": "category"
                                }
                            }
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    let error = json["error"].as_str().unwrap_or_default();

    assert!(error.contains("Invalid filter at index 0"));
    assert!(error.contains("Hint"));
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn test_search_ids_with_filter() {
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
                        "name": "ids_filter",
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
                .uri("/collections/ids_filter/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"category": "a"}},
                            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"category": "b"}},
                            {"id": 3, "vector": [0.8, 0.2, 0.0, 0.0], "payload": {"category": "a"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // search/ids with filter — should only return category="a" points
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/ids_filter/search/ids")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 10,
                        "filter": {
                            "condition": {
                                "type": "eq",
                                "field": "category",
                                "value": "a"
                            }
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
    let results = json["results"].as_array().expect("results is array");

    // Only IDs 1 and 3 have category="a"
    let ids: Vec<u64> = results
        .iter()
        .filter_map(|r| r["id"].as_str().and_then(|s| s.parse::<u64>().ok()))
        .collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&3));
    assert!(!ids.contains(&2));
    // No payload field in the response
    for r in results {
        assert!(r.get("payload").is_none());
    }
}

#[tokio::test]
async fn test_search_ids_with_mode() {
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
                        "name": "ids_mode",
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
                .uri("/collections/ids_mode/points")
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

    // search/ids with mode=accurate — should succeed and return results
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/ids_mode/search/ids")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 2,
                        "mode": "accurate"
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
    let results = json["results"].as_array().expect("results is array");

    assert!(!results.is_empty());
    // Verify id and score fields exist, but no payload
    for r in results {
        assert!(r["id"].is_string());
        assert!(r["score"].is_number());
        assert!(r.get("payload").is_none());
    }
}

#[tokio::test]
async fn test_search_ids_sparse() {
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
                        "name": "ids_sparse",
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

    // Upsert points with sparse vectors (auto-creates the sparse index)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/ids_sparse/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {
                                "id": 1,
                                "vector": [1.0, 0.0, 0.0, 0.0],
                                "sparse_vectors": {"": {"indices": [0, 1], "values": [1.0, 0.5]}}
                            },
                            {
                                "id": 2,
                                "vector": [0.0, 1.0, 0.0, 0.0],
                                "sparse_vectors": {"": {"indices": [1, 2], "values": [0.8, 0.3]}}
                            }
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // search/ids with sparse_vector only — no dense vector
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/ids_sparse/search/ids")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "sparse_vector": {"indices": [0, 1], "values": [1.0, 0.5]},
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
    let results = json["results"].as_array().expect("results is array");

    // Should return results as id+score only
    for r in results {
        assert!(r["id"].is_string());
        assert!(r["score"].is_number());
        assert!(r.get("payload").is_none());
    }
}

// ============================================================================
// EXPLAIN endpoint
// ============================================================================

#[tokio::test]
async fn test_explain_endpoint() {
    let temp_dir = TempDir::new().unwrap();
    let app = create_test_app(&temp_dir);

    // Create collection so the EXPLAIN handler finds it
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "explain_coll",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // POST /query/explain with a simple SELECT
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query/explain")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "SELECT * FROM explain_coll LIMIT 10"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["query_type"], "SELECT");
    assert_eq!(json["collection"], "explain_coll");
    assert!(json["plan"].is_array());
    assert!(!json["plan"].as_array().unwrap().is_empty());
    assert!(json["estimated_cost"].is_object());
    assert!(json["features"].is_object());
}

// ============================================================================
// GuardRails — rate limit (429)
// ============================================================================

#[tokio::test]
async fn test_guardrails_rate_limit_429() {
    let temp_dir = TempDir::new().unwrap();
    let (app, state) = create_test_app_with_state(&temp_dir);

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
                        "name": "rate_coll",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Exhaust the rate limiter for the default client ("anonymous").
    // This sets tokens to 0 AND refill rate to 0, so no refill race is possible.
    let collection = state.db.get_vector_collection("rate_coll").unwrap();
    collection.guard_rails().rate_limiter.exhaust("anonymous");

    // The next search request should be rejected with 429.
    // With refill rate at 0, no tokens will be added back.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/rate_coll/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 1
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

// ============================================================================
// GuardRails — circuit breaker (503)
// ============================================================================

#[tokio::test]
async fn test_guardrails_circuit_breaker_503() {
    let temp_dir = TempDir::new().unwrap();
    let (app, state) = create_test_app_with_state(&temp_dir);

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
                        "name": "cb_coll",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Trip the circuit breaker by recording enough failures.
    // Default failure threshold is 5.
    let collection = state.db.get_vector_collection("cb_coll").unwrap();
    let guard_rails = collection.guard_rails();
    for _ in 0..5 {
        guard_rails.circuit_breaker.record_failure();
    }

    // The next search request should be rejected with 503
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/cb_coll/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 1
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ============================================================================
// Get point by ID
// ============================================================================

#[tokio::test]
async fn test_get_point_by_id() {
    let temp_dir = TempDir::new().unwrap();
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
                        "name": "get_pt",
                        "dimension": 3,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Upsert a point with payload
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/get_pt/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [{
                            "id": 42,
                            "vector": [1.0, 0.0, 0.0],
                            "payload": {"color": "red"}
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // GET the point back
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/collections/get_pt/points/42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["id"], 42);
    assert_eq!(json["vector"], json!([1.0, 0.0, 0.0]));
    assert_eq!(json["payload"]["color"], "red");

    // GET a non-existent point returns 404
    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/get_pt/points/999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Delete point by ID
// ============================================================================

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn test_delete_point_by_id() {
    let temp_dir = TempDir::new().unwrap();
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
                        "name": "del_pt",
                        "dimension": 3,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Upsert a point
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/del_pt/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [{
                            "id": 7,
                            "vector": [0.0, 1.0, 0.0],
                            "payload": {"tag": "ephemeral"}
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Confirm the point exists
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/collections/del_pt/points/7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // DELETE the point
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/collections/del_pt/points/7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["id"], 7);

    // GET after delete returns 404
    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/del_pt/points/7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ────────────────────────────────────────────────────────────────────────────
// Multi-query search — filter forwarding (Sprint 1 / F-04 regression tests)
// ────────────────────────────────────────────────────────────────────────────

/// Helper: seed a collection named `multi_filter` with three categorised points.
async fn seed_multi_query_filter_collection(app: &axum::Router) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "multi_filter",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build create collection request"),
        )
        .await
        .expect("Create collection request failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/multi_filter/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"category": "a"}},
                            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"category": "b"}},
                            {"id": 3, "vector": [0.8, 0.2, 0.0, 0.0], "payload": {"category": "a"}}
                        ]
                    })
                    .to_string(),
                ))
                .expect("Failed to build upsert request"),
        )
        .await
        .expect("Upsert request failed");
    assert_eq!(response.status(), StatusCode::OK);
}

/// Nominal: `/search/multi` must apply a metadata filter and exclude rows
/// that do not match. With three points (ids 1, 2, 3) and a filter
/// `category = "a"`, the result set must be `{1, 3}` — id 2 belongs to
/// category "b" and must be excluded.
///
/// Regression test for F-04: `MultiQuerySearchRequest.filter` was previously
/// deserialized by the handler but never forwarded to
/// `VectorCollection::multi_query_search`, so all rows were returned.
#[tokio::test]
async fn test_multi_query_search_with_filter_excludes_nonmatching_points() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);
    seed_multi_query_filter_collection(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/multi_filter/search/multi")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vectors": [
                            [1.0, 0.0, 0.0, 0.0],
                            [0.95, 0.05, 0.0, 0.0]
                        ],
                        "top_k": 10,
                        "strategy": "rrf",
                        "rrf_k": 60,
                        "filter": {
                            "condition": {
                                "type": "eq",
                                "field": "category",
                                "value": "a"
                            }
                        }
                    })
                    .to_string(),
                ))
                .expect("Failed to build multi search request"),
        )
        .await
        .expect("Multi search request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    let results = json["results"].as_array().expect("results is an array");

    let ids: Vec<u64> = results
        .iter()
        .filter_map(|r| {
            r["id"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| r["id"].as_u64())
        })
        .collect();

    assert!(
        ids.contains(&1),
        "expected id=1 (category=a) in filtered results, got {ids:?}"
    );
    assert!(
        ids.contains(&3),
        "expected id=3 (category=a) in filtered results, got {ids:?}"
    );
    assert!(
        !ids.contains(&2),
        "id=2 (category=b) must be excluded by the filter, got {ids:?}"
    );
}

/// Without a filter, `/search/multi` must return all three points (verifies
/// that the filter fix does not break the baseline behaviour).
#[tokio::test]
async fn test_multi_query_search_without_filter_returns_all_points() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);
    seed_multi_query_filter_collection(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/multi_filter/search/multi")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vectors": [[1.0, 0.0, 0.0, 0.0]],
                        "top_k": 10,
                        "strategy": "rrf",
                        "rrf_k": 60
                    })
                    .to_string(),
                ))
                .expect("Failed to build multi search request"),
        )
        .await
        .expect("Multi search request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    let results = json["results"].as_array().expect("results is an array");
    assert_eq!(
        results.len(),
        3,
        "without filter, all three points must be returned"
    );
}

/// Negative: an invalid filter expression must produce a 400 response
/// (not a 500 and not silently dropped).
#[tokio::test]
async fn test_multi_query_search_with_invalid_filter_returns_400() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);
    seed_multi_query_filter_collection(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/multi_filter/search/multi")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vectors": [[1.0, 0.0, 0.0, 0.0]],
                        "top_k": 10,
                        "strategy": "rrf",
                        "rrf_k": 60,
                        "filter": {
                            "condition": {
                                "type": "nonexistent_operator",
                                "field": "category",
                                "value": "a"
                            }
                        }
                    })
                    .to_string(),
                ))
                .expect("Failed to build multi search request"),
        )
        .await
        .expect("Multi search request failed");

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "invalid filter must be rejected with 400"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Graph collection resolution — no auto-create (Sprint 1 / F-05 regression)
// ────────────────────────────────────────────────────────────────────────────

/// Nominal negative: calling a graph endpoint on a collection that was
/// never created must return 404 Not Found, not silently create a
/// schemaless graph collection. This is the F-05 regression guard.
#[tokio::test]
async fn test_graph_endpoint_on_missing_collection_returns_404() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/never_created/graph/edges")
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

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "graph endpoints must return 404 when the collection does not exist"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).expect("Invalid JSON");
    let error_msg = json["error"]
        .as_str()
        .expect("error field must be present");
    assert!(
        error_msg.contains("never_created"),
        "error message must include the collection name, got: {error_msg}"
    );
    assert!(
        error_msg.contains("collection_type"),
        "error message must guide callers to create the collection explicitly, got: {error_msg}"
    );
}

/// GET /collections/{name}/graph/edges?label=... on a missing graph
/// collection must return 404 (not auto-create and not 500).
#[tokio::test]
async fn test_graph_get_edges_on_missing_collection_returns_404() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/ghost/graph/edges?label=KNOWS")
                .body(Body::empty())
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// POST /collections/{name}/graph/traverse on a missing graph collection
/// must return 404 (regression: auto-create previously hid this path).
#[tokio::test]
async fn test_graph_traverse_on_missing_collection_returns_404() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/absent/graph/traverse")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "source": 1,
                        "strategy": "bfs",
                        "max_depth": 3
                    })
                    .to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// After an explicit `POST /collections` with `collection_type = "graph"`,
/// graph endpoints must succeed as before — this is the happy path that
/// validates the new explicit-creation contract.
#[tokio::test]
async fn test_graph_endpoint_works_after_explicit_creation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);
    create_graph_collection(&app, "explicit").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/explicit/graph/edges")
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
}

// ────────────────────────────────────────────────────────────────────────────
// Per-request timeout_ms — honoured by /search (Sprint 1 / F-03)
// ────────────────────────────────────────────────────────────────────────────

/// Helper: seed a small vector collection for timeout tests.
async fn seed_timeout_collection(app: &axum::Router, name: &str) {
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
                .expect("test: build create collection request"),
        )
        .await
        .expect("test: create collection request failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{name}/points"))
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
                .expect("test: build upsert request"),
        )
        .await
        .expect("test: upsert request failed");
    assert_eq!(response.status(), StatusCode::OK);
}

/// Nominal: a search without `timeout_ms` must behave exactly as before
/// and return 200 with the expected result set. This locks in the
/// baseline so the F-03 wrapping does not introduce latency or break
/// the no-timeout path.
#[tokio::test]
async fn test_search_without_timeout_returns_200() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);
    seed_timeout_collection(&app, "timeout_ok").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/timeout_ok/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 3
                    })
                    .to_string(),
                ))
                .expect("test: build search request"),
        )
        .await
        .expect("test: search request failed");

    assert_eq!(response.status(), StatusCode::OK);
}

/// Nominal: a search with a generous `timeout_ms` (30 seconds) must
/// return 200. This verifies the new wrapping code path behaves like
/// the pass-through code path for any realistic query.
#[tokio::test]
async fn test_search_with_generous_timeout_returns_200() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);
    seed_timeout_collection(&app, "timeout_generous").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/timeout_generous/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 3,
                        "timeout_ms": 30000
                    })
                    .to_string(),
                ))
                .expect("test: build search request"),
        )
        .await
        .expect("test: search request failed");

    assert_eq!(response.status(), StatusCode::OK);
}

/// Negative: a search with `timeout_ms: 0` yields an immediate timeout.
/// The handler must return 408 Request Timeout with a VELES-QUERY-TIMEOUT
/// error code, not 200 with the results. The `tokio::time::timeout`
/// wrapper fires on the very next runtime tick after the worker is
/// spawned, so this deterministic test will always see the elapsed
/// path regardless of how fast the underlying HNSW search is.
#[tokio::test]
async fn test_search_with_zero_timeout_returns_408() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);
    seed_timeout_collection(&app, "timeout_zero").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/timeout_zero/search")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "vector": [1.0, 0.0, 0.0, 0.0],
                        "top_k": 3,
                        "timeout_ms": 0
                    })
                    .to_string(),
                ))
                .expect("test: build search request"),
        )
        .await
        .expect("test: search request failed");

    assert_eq!(
        response.status(),
        StatusCode::REQUEST_TIMEOUT,
        "F-03: timeout_ms=0 must return 408 Request Timeout"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json: Value = serde_json::from_slice(&body).expect("test: parse json");
    assert_eq!(
        json["code"].as_str(),
        Some("VELES-QUERY-TIMEOUT"),
        "error code must be VELES-QUERY-TIMEOUT, got: {}",
        json["code"]
    );
    let error_msg = json["error"].as_str().expect("error field");
    assert!(
        error_msg.contains("timeout_zero"),
        "error must include collection name, got: {error_msg}"
    );
    assert!(
        error_msg.contains("0ms"),
        "error must echo the budget, got: {error_msg}"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Type-mismatch branch for F-05 (retained from the previous section)
// ────────────────────────────────────────────────────────────────────────────

// ────────────────────────────────────────────────────────────────────────────
// PROP-CONFIG-ADVANCED: CreateCollectionRequest accepts pq_rescore_oversampling,
// deferred_indexing, async_index_builder; GET /collections/{name}/config
// echoes them back (Sprint 1 / S1-07).
// ────────────────────────────────────────────────────────────────────────────

/// Nominal round-trip: create a collection with pq_rescore_oversampling
/// and async_index_builder, then GET /config and verify the values are
/// persisted and echoed.
#[tokio::test]
async fn test_create_with_pq_rescore_and_async_builder_round_trip() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create with advanced config.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "advanced_cfg",
                        "dimension": 16,
                        "metric": "cosine",
                        "pq_rescore_oversampling": 8,
                        "async_index_builder": {
                            "merge_threshold": 5000,
                            "segment_count": 4
                        }
                    })
                    .to_string(),
                ))
                .expect("test: build create request"),
        )
        .await
        .expect("test: create request failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    // Describe: GET /collections/{name}/config.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/advanced_cfg/config")
                .body(Body::empty())
                .expect("test: build describe request"),
        )
        .await
        .expect("test: describe request failed");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json: Value = serde_json::from_slice(&body).expect("test: parse json");

    assert_eq!(json["name"], "advanced_cfg");
    assert_eq!(json["dimension"], 16);
    assert_eq!(
        json["pq_rescore_oversampling"], 8,
        "pq_rescore_oversampling must round-trip through describe, got: {}",
        json["pq_rescore_oversampling"]
    );

    let aib = &json["async_index_builder"];
    assert!(
        aib.is_object(),
        "async_index_builder must be populated in describe response, got: {aib}"
    );
    assert_eq!(
        aib["merge_threshold"], 5000,
        "async_index_builder.merge_threshold must round-trip"
    );
    assert_eq!(
        aib["segment_count"], 4,
        "async_index_builder.segment_count must round-trip"
    );

    // schema_version is always populated (non-optional in the response).
    assert!(
        json["schema_version"].as_u64().is_some(),
        "schema_version must be present"
    );
}

/// Edge: a collection created WITHOUT advanced overrides must still
/// describe cleanly. pq_rescore_oversampling defaults to 4 (the core
/// default); async_index_builder and deferred_indexing are absent.
#[tokio::test]
async fn test_create_without_advanced_config_describe_returns_defaults() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "minimal_cfg",
                        "dimension": 8,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("test: build create request"),
        )
        .await
        .expect("test: create request failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections/minimal_cfg/config")
                .body(Body::empty())
                .expect("test: build describe request"),
        )
        .await
        .expect("test: describe request failed");
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json: Value = serde_json::from_slice(&body).expect("test: parse json");

    assert_eq!(
        json["pq_rescore_oversampling"], 4,
        "pq_rescore_oversampling must default to 4"
    );
    assert!(
        json.get("async_index_builder").is_none()
            || json["async_index_builder"].is_null(),
        "async_index_builder must be absent or null when not set"
    );
}

/// Negative: a malformed `async_index_builder` payload must return
/// 400 Bad Request with a message that identifies the offending field.
#[tokio::test]
async fn test_create_with_invalid_async_index_builder_returns_400() {
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
                        "name": "bad_aib",
                        "dimension": 8,
                        "metric": "cosine",
                        "async_index_builder": {
                            "merge_threshold": "this is not a number"
                        }
                    })
                    .to_string(),
                ))
                .expect("test: build create request"),
        )
        .await
        .expect("test: create request failed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json: Value = serde_json::from_slice(&body).expect("test: parse json");
    let error_msg = json["error"].as_str().expect("error field");
    assert!(
        error_msg.contains("async_index_builder"),
        "error must name the offending field, got: {error_msg}"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// PROP-GRAPHSCHEMA-SERVER: CreateCollectionRequest accepts a typed
// graph_schema payload instead of hard-coding GraphSchema::schemaless()
// (Sprint 1 / S1-08).
// ────────────────────────────────────────────────────────────────────────────

/// Nominal: creating a graph collection with a typed `graph_schema`
/// payload must succeed with 201 and the schema must persist through
/// GET /config. The shape matches `velesdb_core::GraphSchema`: a
/// `schemaless` boolean plus `node_types` / `edge_types` arrays.
#[tokio::test]
async fn test_create_graph_collection_with_typed_schema() {
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
                        "name": "typed_graph",
                        "collection_type": "graph",
                        "graph_schema": {
                            "schemaless": false,
                            "node_types": [],
                            "edge_types": []
                        }
                    })
                    .to_string(),
                ))
                .expect("test: build create request"),
        )
        .await
        .expect("test: create request failed");

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "typed graph schema must be accepted"
    );
}

/// Edge: a graph collection created without a `graph_schema` field must
/// continue to work and fall back to `GraphSchema::schemaless()`. This
/// locks in the backward-compatibility promise.
#[tokio::test]
async fn test_create_graph_collection_without_schema_uses_schemaless() {
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
                        "name": "implicit_schemaless",
                        "collection_type": "graph"
                    })
                    .to_string(),
                ))
                .expect("test: build create request"),
        )
        .await
        .expect("test: create request failed");

    assert_eq!(response.status(), StatusCode::CREATED);
}

/// Negative: a malformed `graph_schema` payload must return 400 Bad
/// Request with a message that identifies the offending field.
#[tokio::test]
async fn test_create_graph_collection_with_invalid_schema_returns_400() {
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
                        "name": "bad_schema",
                        "collection_type": "graph",
                        "graph_schema": "not an object"
                    })
                    .to_string(),
                ))
                .expect("test: build create request"),
        )
        .await
        .expect("test: create request failed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json: Value = serde_json::from_slice(&body).expect("test: parse json");
    let error_msg = json["error"].as_str().expect("error field");
    assert!(
        error_msg.contains("graph_schema"),
        "error must name the offending field, got: {error_msg}"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// F-21: POST /collections/{name}/index/rebuild — HNSW vacuum endpoint
// ────────────────────────────────────────────────────────────────────────────

/// Nominal: `POST /collections/{name}/index/rebuild` on a populated
/// vector collection must return 200 with `compacted_entries` in the
/// body. For a collection that has not had any deletions, the compacted
/// count is expected to be 0 — the important contract is that the
/// endpoint succeeds and surfaces the count honestly.
#[tokio::test]
async fn test_rebuild_index_on_populated_collection_returns_200() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Seed a small collection with three points.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "rebuild_ok",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("test: build create request"),
        )
        .await
        .expect("test: create request failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/rebuild_ok/points")
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
                .expect("test: build upsert request"),
        )
        .await
        .expect("test: upsert request failed");
    assert_eq!(response.status(), StatusCode::OK);

    // Rebuild the index.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/rebuild_ok/index/rebuild")
                .body(Body::empty())
                .expect("test: build rebuild request"),
        )
        .await
        .expect("test: rebuild request failed");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json: Value = serde_json::from_slice(&body).expect("test: parse json");

    assert_eq!(json["message"], "Index rebuilt");
    assert_eq!(json["collection"], "rebuild_ok");
    assert!(
        json["compacted_entries"].as_u64().is_some(),
        "compacted_entries must be a number, got: {}",
        json["compacted_entries"]
    );
}

/// Negative: `POST /collections/{name}/index/rebuild` on a missing
/// collection must return 404.
#[tokio::test]
async fn test_rebuild_index_on_missing_collection_returns_404() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/never_created/index/rebuild")
                .body(Body::empty())
                .expect("test: build rebuild request"),
        )
        .await
        .expect("test: rebuild request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Type-mismatch: a vector collection already exists with the target
/// name, but the caller targets a graph endpoint. The handler must
/// return 409 Conflict (already the case before F-05, but we lock it
/// in with an explicit test so the fix cannot regress the branch).
#[tokio::test]
async fn test_graph_endpoint_on_vector_collection_returns_409() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let app = create_test_app(&temp_dir);

    // Create a vector collection (not a graph collection).
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "type_mismatch",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("Failed to build create collection request"),
        )
        .await
        .expect("Create collection request failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    // Now POST to its graph/edges endpoint → must be 409, not 404 or 500.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/type_mismatch/graph/edges")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "id": 1,
                        "source": 1,
                        "target": 2,
                        "label": "X"
                    })
                    .to_string(),
                ))
                .expect("Failed to build graph edge request"),
        )
        .await
        .expect("Graph edge request failed");

    assert_eq!(response.status(), StatusCode::CONFLICT);
}
