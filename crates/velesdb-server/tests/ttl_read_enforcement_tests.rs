//! Durable point-TTL enforcement on REST read surfaces (AM-1).
//!
//! `PATCH /collections/{name}/points/{id}/ttl` with `ttl_seconds: 0` expires
//! a point immediately: it must vanish from GET point, POST search, scroll,
//! and `/query` SELECT, and refreshing its TTL again must return 404.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use common::create_test_app;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

/// Sends one JSON request and returns `(status, parsed body)`.
async fn send_json(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            builder = builder.header("Content-Type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("request failed");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("parse JSON body")
    };
    (status, parsed)
}

/// Extracts the `id` fields of an array of result objects as strings
/// (search/scroll serialize ids as strings, `/query` as integers).
fn ids_of(items: &Value) -> Vec<String> {
    items
        .as_array()
        .expect("results array")
        .iter()
        .map(|item| match &item["id"] {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect()
}

/// Creates `ttl_coll` with live point 1 and point 2, then expires point 2
/// via `PATCH …/points/2/ttl {ttl_seconds: 0}`.
async fn setup_expired_point(app: &Router) {
    let (status, _) = send_json(
        app,
        "POST",
        "/collections",
        Some(json!({"name": "ttl_coll", "dimension": 4, "metric": "cosine"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = send_json(
        app,
        "POST",
        "/collections/ttl_coll/points",
        Some(json!({
            "points": [
                {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"category": "tech"}},
                {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"category": "tech"}}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send_json(
        app,
        "PATCH",
        "/collections/ttl_coll/points/2/ttl",
        Some(json!({"ttl_seconds": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "ttl_seconds=0 is accepted");
}

#[tokio::test]
async fn test_expired_point_get_returns_404() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    setup_expired_point(&app).await;

    let (status, _) = send_json(&app, "GET", "/collections/ttl_coll/points/2", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expired point must 404");

    let (status, _) = send_json(&app, "GET", "/collections/ttl_coll/points/1", None).await;
    assert_eq!(status, StatusCode::OK, "live point stays readable");
}

#[tokio::test]
async fn test_expired_point_absent_from_search() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    setup_expired_point(&app).await;

    let (status, body) = send_json(
        &app,
        "POST",
        "/collections/ttl_coll/search",
        Some(json!({"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 10})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ids = ids_of(&body["results"]);
    assert!(ids.contains(&"1".to_string()), "live point found: {ids:?}");
    assert!(
        !ids.contains(&"2".to_string()),
        "expired point must be absent from search: {ids:?}"
    );
}

#[tokio::test]
async fn test_expired_point_absent_from_scroll() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    setup_expired_point(&app).await;

    let (status, body) = send_json(
        &app,
        "POST",
        "/collections/ttl_coll/points/scroll",
        Some(json!({"batch_size": 100})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ids = ids_of(&body["points"]);
    assert_eq!(ids, vec!["1"], "scroll must skip the expired point");
}

#[tokio::test]
async fn test_expired_point_absent_from_query_select() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    setup_expired_point(&app).await;

    let (status, body) = send_json(
        &app,
        "POST",
        "/query",
        Some(json!({
            "query": "SELECT * FROM ttl_coll WHERE category = 'tech' LIMIT 10",
            "params": {}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ids = ids_of(&body["results"]);
    assert!(ids.contains(&"1".to_string()), "live point found: {ids:?}");
    assert!(
        !ids.contains(&"2".to_string()),
        "expired point must be absent from SELECT: {ids:?}"
    );
}

#[tokio::test]
async fn test_refreshing_expired_point_returns_404() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    setup_expired_point(&app).await;

    // Refreshing an expired point must not resurrect it.
    let (status, _) = send_json(
        &app,
        "PATCH",
        "/collections/ttl_coll/points/2/ttl",
        Some(json!({"ttl_seconds": 3600})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "re-PATCH of an expired point must 404"
    );
}
