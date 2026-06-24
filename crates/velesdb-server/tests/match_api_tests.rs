//! E2E tests for /match endpoint (EPIC-058 US-007).
//!
//! Tests the hybrid MATCH + similarity + property projection API.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{create_graph_collection, create_test_app};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

/// A non-MATCH query (e.g. a SELECT) sent to `/match` is a query-shape client
/// mistake and now surfaces the canonical `VELES-010` code with a 400 status,
/// not the former bespoke `NOT_MATCH_QUERY` string.
#[tokio::test]
async fn test_match_not_match_query_returns_veles_010() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    create_graph_collection(&app, "social").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/social/match")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({ "query": "SELECT * FROM social" }).to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json: Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(json["code"], "VELES-010");
}
