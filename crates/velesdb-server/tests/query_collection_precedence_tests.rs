//! The collection a top-level `MATCH` — the one statement without `FROM` —
//! runs on, over `/query`: the body's `collection` fills `params._collection`
//! when the caller set none, and an explicit `_collection` wins over it
//! (core's `velesql::with_collection_param` / `collection_target`, whose own
//! cases are `velesdb-core/src/velesql/collection_target_tests.rs`).
//!
//! The `VelesQL` grammar's `select_stmt` requires `FROM`, so an aggregate
//! never reads the key: here its `FROM` is shown to decide over both the body
//! and the param, on `/query` and `/aggregate`.
//!
//! The fixture is two collections of different sizes, `small` (2 rows) and
//! `large` (3 rows), so the row count tells which one a statement ran on.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

async fn post_json(app: &axum::Router, uri: &str, body: &Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("test: build request"),
        )
        .await
        .expect("test: request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn seed(app: &axum::Router, name: &str, rows: u64) {
    let (status, body) = post_json(
        app,
        "/collections",
        &json!({ "name": name, "dimension": 2, "metric": "cosine" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let points: Vec<Value> = (1..=rows)
        .map(|id| json!({ "id": id, "vector": [1.0, 0.0], "payload": {} }))
        .collect();
    let (status, body) = post_json(
        app,
        &format!("/collections/{name}/points"),
        &json!({ "points": points }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn setup() -> (TempDir, axum::Router) {
    let dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&dir);
    seed(&app, "small", 2).await;
    seed(&app, "large", 3).await;
    (dir, app)
}

const MATCH_ALL: &str = "MATCH (n) RETURN n LIMIT 10";

/// The number of rows a `MATCH` on `/query` returned.
fn match_rows(body: &Value) -> usize {
    body["results"].as_array().map_or(0, Vec::len)
}

/// GIVEN a top-level MATCH on `/query` naming `small` in the body
/// WHEN `params._collection` names `large`
/// THEN the MATCH runs on `large`: the explicit key wins.
#[tokio::test]
async fn match_on_query_an_explicit_collection_param_wins_over_the_body() {
    let (_dir, app) = setup().await;

    let (status, body) = post_json(
        &app,
        "/query",
        &json!({ "query": MATCH_ALL, "collection": "small", "params": { "_collection": "large" } }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(match_rows(&body), 3, "{body}");
}

/// GIVEN a top-level MATCH on `/query` with no body `collection`
/// WHEN `params._collection` names `large`
/// THEN the MATCH runs on `large`, as it would with the body field.
#[tokio::test]
async fn match_on_query_is_addressed_by_the_collection_param_alone() {
    let (_dir, app) = setup().await;

    let (status, body) = post_json(
        &app,
        "/query",
        &json!({ "query": MATCH_ALL, "params": { "_collection": "large" } }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(match_rows(&body), 3, "{body}");
}

/// GIVEN a top-level MATCH on `/query` naming no collection, in the body or
///      in `params._collection` (an empty body `collection` is none)
/// THEN it is refused with `VELESQL_MISSING_COLLECTION`.
#[tokio::test]
async fn match_on_query_naming_no_collection_is_refused() {
    let (_dir, app) = setup().await;

    for request in [
        json!({ "query": MATCH_ALL }),
        json!({ "query": MATCH_ALL, "collection": "" }),
    ] {
        let (status, body) = post_json(&app, "/query", &request).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{request}: {body}"
        );
        assert_eq!(
            body["error"]["code"], "VELESQL_MISSING_COLLECTION",
            "{request}: {body}"
        );
    }
}

/// GIVEN a top-level MATCH whose `params._collection` is no collection name
///      (empty, or not a string), with or without a body `collection`
/// THEN it is refused by core's rule (`velesql::collection_param`): a
///      `VELESQL_PARAMETER_ERROR` naming `_collection`, the explicit key
///      winning over the body as it does when valid.
#[tokio::test]
async fn match_on_query_with_an_invalid_collection_param_is_a_parameter_error() {
    let (_dir, app) = setup().await;

    for bad in [json!(""), json!(7)] {
        for request in [
            json!({ "query": MATCH_ALL, "params": { "_collection": bad } }),
            json!({ "query": MATCH_ALL, "collection": "small", "params": { "_collection": bad } }),
        ] {
            let (status, body) = post_json(&app, "/query", &request).await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{request}: {body}"
            );
            assert_eq!(
                body["error"]["code"], "VELESQL_PARAMETER_ERROR",
                "{request}: {body}"
            );
            assert_eq!(
                body["error"]["details"]["parameter"], "_collection",
                "{request}: {body}"
            );
        }
    }
}

/// GIVEN an aggregate with a `FROM`
/// THEN its `FROM` decides, whatever the body and `_collection` name.
#[tokio::test]
async fn aggregate_with_from_ignores_the_body_and_the_param() {
    let (_dir, app) = setup().await;

    for endpoint in ["/query", "/aggregate"] {
        let (status, body) = post_json(
            &app,
            endpoint,
            &json!({
                "query": "SELECT COUNT(*) AS n FROM small",
                "collection": "large",
                "params": { "_collection": "large" }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{endpoint}: {body}");
        assert_eq!(body["result"]["n"], 2, "{endpoint}: {body}");
    }
}
