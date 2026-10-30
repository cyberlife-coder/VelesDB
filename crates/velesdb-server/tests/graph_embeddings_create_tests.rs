//! `POST /collections` with `collection_type: "graph"` honours `dimension`
//! and `metric`: the collection gets node embeddings of that dimension,
//! searched with that metric. The handler used to call
//! `create_graph_collection(name, schema)` whatever the body said, so a
//! graph created with `dimension: 4, metric: "euclidean"` had no embeddings
//! at all, and the two fields were dropped without a word.
//!
//! Driven through `routes::api_routes()`, the production route table.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

fn app(temp_dir: &TempDir) -> Router {
    let (_, state) = common::create_test_app_with_state(temp_dir);
    velesdb_server::routes::api_routes().with_state(state)
}

async fn call(app: &Router, method: &str, uri: &str, body: &Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
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
        .expect("test: body");
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn create(app: &Router, body: &Value) {
    let (status, json) = call(app, "POST", "/collections", body).await;
    assert_eq!(status, StatusCode::CREATED, "{json}");
}

/// Two nodes whose order differs by metric for the query `[1, 0, 0, 0]`:
/// node 1 points the same way but is far away (cosine's favourite), node 2
/// is close but at 45 degrees (euclidean's favourite).
async fn insert_nodes(app: &Router) {
    let (status, json) = call(
        app,
        "POST",
        "/query",
        &json!({
            "query": "INSERT INTO kg (id, vector, name) VALUES (1, $far, 'far'), (2, $near, 'near')",
            "params": { "far": [10.0, 0.0, 0.0, 0.0], "near": [1.0, 1.0, 0.0, 0.0] }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
}

async fn search_ids(app: &Router, vector: &[f32]) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        "/collections/kg/graph/search",
        &json!({ "vector": vector, "top_k": 2 }),
    )
    .await
}

#[tokio::test]
async fn test_graph_created_with_dimension_and_metric_searches_with_them() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app(&temp_dir);
    create(
        &app,
        &json!({ "name": "kg", "collection_type": "graph", "dimension": 4, "metric": "euclidean" }),
    )
    .await;

    let (status, info) = call(&app, "GET", "/collections/kg", &Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["dimension"], 4, "{info}");
    assert_eq!(info["metric"], "euclidean", "{info}");

    insert_nodes(&app).await;
    let (status, found) = search_ids(&app, &[1.0, 0.0, 0.0, 0.0]).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let ids: Vec<&str> = found["results"]
        .as_array()
        .expect("test: results")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert_eq!(ids, ["2", "1"], "euclidean ranks the near node first: {found}");

    let (status, body) = search_ids(&app, &[1.0, 0.0, 0.0]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a 3-d query on a 4-d graph: {body}");
}

/// Positive control for the metric: the same nodes under cosine rank the
/// other way, so the order above is the metric, not the data.
#[tokio::test]
async fn test_graph_created_with_cosine_ranks_by_direction() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app(&temp_dir);
    create(
        &app,
        &json!({ "name": "kg", "collection_type": "graph", "dimension": 4, "metric": "cosine" }),
    )
    .await;
    insert_nodes(&app).await;

    let (status, found) = search_ids(&app, &[1.0, 0.0, 0.0, 0.0]).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["results"][0]["id"], "1", "{found}");
}

/// Without `dimension` a graph collection still has no embeddings, as before.
#[tokio::test]
async fn test_graph_created_without_dimension_has_no_embeddings() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app(&temp_dir);
    create(&app, &json!({ "name": "kg", "collection_type": "graph" })).await;

    let (status, body) = search_ids(&app, &[1.0, 0.0, 0.0, 0.0]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
