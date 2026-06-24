#![allow(clippy::doc_markdown)]
//! Regression tests for backlog #13: graph/MATCH REST handlers must surface
//! the canonical `VELES-XXX` error codes and correct 4xx statuses (via
//! `auto_core_error_response`) instead of invented codes / blanket-500s.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{create_graph_collection, create_test_app};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

async fn body_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&body).expect("valid JSON")
}

/// POST /match to a collection that does not exist must return 404 with the
/// canonical `VELES-002 CollectionNotFound` code (not `COLLECTION_NOT_FOUND`).
#[tokio::test]
async fn test_match_missing_collection_returns_veles_002() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/ghost/match")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({ "query": "MATCH (n) RETURN n" }).to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = body_json(response).await;
    assert_eq!(json["code"], "VELES-002");
}

/// POST /match whose WHERE references an unbound `$v` parameter must surface
/// the core `Error::Query` code (`VELES-010` post-#1212) with a 400 status,
/// not a blanket 500 / invented `EXECUTION_ERROR`. A request `vector` plus a
/// seeded node routes through `execute_match_with_similarity`, whose WHERE
/// evaluation resolves `$v` and fails deterministically when it is absent.
#[tokio::test]
async fn test_match_missing_param_returns_veles_010() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_vector_node(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/people/match")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "query": "MATCH (a:Person) WHERE similarity(a.vec, $v) > 0.1 RETURN a",
                        "vector": [1.0, 0.0, 0.0, 0.0]
                    })
                    .to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "missing bind-param is a client error (400), not a server 500"
    );
    let json = body_json(response).await;
    assert_eq!(json["code"], "VELES-010");
}

/// Creates a `people` vector collection with one labeled `Person` node so the
/// `/match` similarity WHERE path has a candidate to evaluate.
async fn seed_vector_node(app: &axum::Router) {
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({ "name": "people", "dimension": 4, "metric": "cosine" }).to_string(),
                ))
                .expect("build create request"),
        )
        .await
        .expect("create request");
    assert_eq!(create.status(), StatusCode::CREATED);

    let upsert = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/people/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [{
                            "id": 1,
                            "vector": [1.0, 0.0, 0.0, 0.0],
                            "payload": { "_labels": ["Person"], "vec": [1.0, 0.0, 0.0, 0.0] }
                        }]
                    })
                    .to_string(),
                ))
                .expect("build upsert request"),
        )
        .await
        .expect("upsert request");
    assert_eq!(upsert.status(), StatusCode::OK);
}

/// Adding an edge whose ID already exists must return 409 Conflict with the
/// canonical `VELES-019 EdgeExists` code (not a generic 500 string).
#[tokio::test]
async fn test_duplicate_edge_returns_veles_019() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    create_graph_collection(&app, "social").await;

    let edge = json!({
        "id": 1,
        "source": 10,
        "target": 20,
        "label": "KNOWS"
    });

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/social/graph/edges")
                .header("Content-Type", "application/json")
                .body(Body::from(edge.to_string()))
                .expect("build request"),
        )
        .await
        .expect("request");
    assert_eq!(first.status(), StatusCode::CREATED);

    let dup = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/social/graph/edges")
                .header("Content-Type", "application/json")
                .body(Body::from(edge.to_string()))
                .expect("build request"),
        )
        .await
        .expect("request");

    assert_eq!(dup.status(), StatusCode::CONFLICT);
    let json = body_json(dup).await;
    assert_eq!(json["code"], "VELES-019");
}
