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

/// Sends `body` to `uri` and returns the status and the JSON body.
async fn post_json(app: &axum::Router, uri: &str, body: &Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("build request"),
        )
        .await
        .expect("request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

/// Creates the euclidean collection `l2` with three points at distances
/// 0, 3 and 5 from the query `[0, 0]`.
async fn seed_euclidean(app: &axum::Router) {
    let (status, body) = post_json(
        app,
        "/collections",
        &json!({ "name": "l2", "dimension": 2, "metric": "euclidean" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = post_json(
        app,
        "/collections/l2/points",
        &json!({ "points": [
            { "id": 1, "vector": [0.0, 0.0], "payload": {} },
            { "id": 2, "vector": [3.0, 0.0], "payload": {} },
            { "id": 3, "vector": [0.0, 5.0], "payload": {} }
        ] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

const MATCH_ALL: &str = "MATCH (n) RETURN n LIMIT 10";

/// The scores of a `/match` response, in response order.
fn scores(body: &Value) -> Vec<f64> {
    body["results"]
        .as_array()
        .expect("results array")
        .iter()
        .map(|row| row["score"].as_f64().expect("scored row"))
        .collect()
}

/// GIVEN a euclidean collection
/// WHEN `/match` carries a query vector and no threshold
/// THEN every row is scored and returned, nearest first: an omitted threshold
///      is no threshold, not `0.0` (which kept only exact duplicates).
#[tokio::test]
async fn test_match_without_threshold_keeps_every_row_on_a_distance_metric() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_euclidean(&app).await;

    let (status, body) = post_json(
        &app,
        "/collections/l2/match",
        &json!({ "query": MATCH_ALL, "vector": [0.0, 0.0] }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(scores(&body), vec![0.0, 3.0, 5.0], "{body}");
}

/// GIVEN a euclidean collection
/// WHEN `/match` carries a distance threshold above 1
/// THEN it is accepted and keeps the rows at most that far.
#[tokio::test]
async fn test_match_accepts_a_distance_threshold_above_one() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_euclidean(&app).await;

    let (status, body) = post_json(
        &app,
        "/collections/l2/match",
        &json!({ "query": MATCH_ALL, "vector": [0.0, 0.0], "threshold": 4.0 }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(scores(&body), vec![0.0, 3.0], "{body}");
}

/// GIVEN a euclidean collection
/// WHEN `/match` carries a threshold and no query vector
/// THEN it is refused with `VELES-010`: without a vector no row is scored,
///      so the threshold would be dropped without a word. Core's rule
///      (`MatchScoring::new`), which every MATCH surface applies.
#[tokio::test]
async fn test_match_refuses_a_threshold_without_a_vector() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_euclidean(&app).await;

    let (status, body) = post_json(
        &app,
        "/collections/l2/match",
        &json!({ "query": MATCH_ALL, "threshold": 4.0 }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "VELES-010", "{body}");
}

/// WHEN `/match` carries a threshold that is no finite `f32` — a `NaN`
///      literal, or `1e39`, which overflows one (JSON has no infinity)
/// THEN the request is refused before it reaches the engine.
#[tokio::test]
async fn test_match_refuses_a_non_finite_threshold() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_euclidean(&app).await;

    for threshold in ["NaN", "1e39", "-1e39"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/collections/l2/match")
                    .header("Content-Type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"query": "{MATCH_ALL}", "vector": [0.0, 0.0], "threshold": {threshold}}}"#
                    )))
                    .expect("build request"),
            )
            .await
            .expect("request");
        assert!(
            response.status().is_client_error(),
            "threshold {threshold} must be refused, got {}",
            response.status()
        );
    }
}
