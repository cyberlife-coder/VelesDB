//! A graph collection reports one size on every REST surface: its node
//! count, every stored node with an embedding or not.
//!
//! `GET /collections/{name}`, `/sanity`, `/config` and `/diagnostics` read
//! the vector store's `point_count`, while `/empty` and `DESCRIBE
//! COLLECTION` counted the nodes, so a graph without embeddings holding two
//! nodes answered `point_count: 0` beside `is_empty: false`. What is about
//! vectors (`has_vectors`, `search_ready`) still reads the embeddings.
//!
//! Driven through `routes::api_routes()`, the production route table.

mod common;

use axum::{http::StatusCode, Router};
use serde_json::{json, Value};
use tempfile::TempDir;

fn app(temp_dir: &TempDir) -> Router {
    let (_, state) = common::create_test_app_with_state(temp_dir);
    velesdb_server::routes::api_routes().with_state(state)
}

async fn get(app: &Router, uri: &str) -> Value {
    let (status, body) = common::call_json(app, "GET", uri, &Value::Null).await;
    assert_eq!(status, StatusCode::OK, "GET {uri}: {body}");
    body
}

/// The `point_count` DESCRIBE COLLECTION reports for `name`.
async fn described_point_count(app: &Router, name: &str) -> Value {
    let (status, body) = common::call_json(
        app,
        "POST",
        "/query",
        &json!({ "query": format!("DESCRIBE COLLECTION {name}") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["results"][0]["point_count"].clone()
}

#[tokio::test]
async fn a_payload_only_graph_reports_its_node_count_everywhere() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = app(&temp_dir);
    common::create_graph_collection(&app, "kg").await;
    common::create_graph_node(&app, "kg", 1).await;
    common::create_graph_node(&app, "kg", 2).await;

    let info = get(&app, "/collections/kg").await;
    let sanity = get(&app, "/collections/kg/sanity").await;
    let config = get(&app, "/collections/kg/config").await;
    let diagnostics = get(&app, "/collections/kg/diagnostics").await;
    let empty = get(&app, "/collections/kg/empty").await;

    assert_eq!(empty["is_empty"], json!(false), "{empty}");
    for (surface, count) in [
        ("GET /collections/kg", &info["point_count"]),
        ("/sanity", &sanity["point_count"]),
        ("/config", &config["point_count"]),
        ("/diagnostics", &diagnostics["point_count"]),
        ("DESCRIBE", &described_point_count(&app, "kg").await),
    ] {
        assert_eq!(count, &json!(2), "{surface} must report the 2 nodes");
    }
    assert_eq!(sanity["is_empty"], json!(false), "{sanity}");

    // No node carries an embedding: nothing to search.
    for (surface, checks) in [
        ("/sanity", &sanity["checks"]),
        ("/diagnostics", &diagnostics),
    ] {
        assert_eq!(checks["has_vectors"], json!(false), "{surface}: {checks}");
        assert_eq!(checks["search_ready"], json!(false), "{surface}: {checks}");
    }
}

/// ANALYZE counts a graph's rows as its nodes: `ANALYZE kg` and `GET
/// /collections/kg/stats` report the 2 nodes `GET /collections/kg` does, not
/// the 0 embeddings the planner used to cost the graph by.
#[tokio::test]
async fn a_payload_only_graph_s_statistics_count_its_nodes() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = app(&temp_dir);
    common::create_graph_collection(&app, "kg").await;
    common::create_graph_node(&app, "kg", 1).await;
    common::create_graph_node(&app, "kg", 2).await;

    let (status, analyze_body) =
        common::call_json(&app, "POST", "/query", &json!({ "query": "ANALYZE kg" })).await;
    assert_eq!(status, StatusCode::OK, "{analyze_body}");
    let kg_stats = get(&app, "/collections/kg/stats").await;

    assert_eq!(kg_stats["row_count"], json!(2), "{kg_stats}");
    assert_eq!(
        analyze_body["results"][0]["row_count"],
        json!(2),
        "{analyze_body}"
    );
}
