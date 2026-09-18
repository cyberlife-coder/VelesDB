#![allow(clippy::doc_markdown)]
//! Coverage-focused handler tests for VelesDB 3.0.0 new code paths.
//!
//! Targets previously-uncovered branches in:
//! - `handlers/search/multi.rs` — fusion-strategy parsing (every arm),
//!   per-vector dimension validation (the indexed 400), and the
//!   rate-limited (429) preamble path shared by `/search/multi` and
//!   `/search/multi/ids`.
//! - `handlers/search/mod.rs` — the operational counters
//!   `search_request_pre_check` records for `/search` and `/search/ids`
//!   (vector-query counter, and `status="rate_limited"` on a 429).
//! - `handlers/graph/handlers.rs` — `build_edge` rejection of non-object
//!   /non-null `properties` (400) for both the single-edge and batch
//!   endpoints.
//!
//! These mirror the existing integration-test harness in
//! `tests/common/mod.rs` (oneshot against a router backed by a temp
//! `Database`).

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{
    create_graph_collection, create_graph_node, create_test_app, create_test_app_with_state,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

/// POST helper: builds a JSON request and runs it against a clone of the app.
async fn post(app: &axum::Router, uri: &str, body: Value) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed")
}

/// Reads the response body as JSON.
async fn read_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    serde_json::from_slice(&body).expect("test: valid JSON")
}

/// Asserts a 400 response carrying the `build_edge` properties-shape error.
async fn assert_properties_shape_error(resp: axum::response::Response) {
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = read_json(resp).await;
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Properties must be an object or null"),
        "expected the properties-shape error, got {json:?}"
    );
}

/// Creates a 4-d cosine collection named `name` and seeds two points so
/// fusion actually has candidates to rank.
async fn seed_vector_collection(app: &axum::Router, name: &str) {
    let resp = post(
        app,
        "/collections",
        json!({"name": name, "dimension": 4, "metric": "cosine"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "test setup: create");

    let resp = post(
        app,
        &format!("/collections/{name}/points"),
        json!({
            "points": [
                {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
                {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]}
            ]
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "test setup: upsert");
}

// ============================================================================
// multi.rs — parse_fusion_strategy: every valid arm returns 200
// ============================================================================

#[tokio::test]
async fn test_multi_query_search_all_valid_strategies_ok() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_vector_collection(&app, "fusion_ok").await;

    // Covers each match arm of `parse_fusion_strategy` including aliases.
    for strategy in [
        "average",
        "avg",
        "maximum",
        "max",
        "rrf",
        "weighted",
        "relative_score",
        "rsf",
    ] {
        let resp = post(
            &app,
            "/collections/fusion_ok/search/multi",
            json!({
                "vectors": [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
                "top_k": 2,
                "strategy": strategy
            }),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "strategy '{strategy}' must return 200"
        );
    }
}

// ============================================================================
// multi.rs — parse_fusion_strategy: unknown strategy -> 400 (the `_` arm)
// ============================================================================

#[tokio::test]
async fn test_multi_query_search_invalid_strategy_returns_400() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_vector_collection(&app, "fusion_bad").await;

    let resp = post(
        &app,
        "/collections/fusion_bad/search/multi",
        json!({
            "vectors": [[1.0, 0.0, 0.0, 0.0]],
            "top_k": 2,
            "strategy": "definitely_not_a_strategy"
        }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = read_json(resp).await;
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Invalid strategy"),
        "expected an 'Invalid strategy' error, got {json:?}"
    );
}

/// The ids-only endpoint shares `prepare_multi_query`, so the unknown
/// strategy must also be rejected there.
#[tokio::test]
async fn test_multi_query_search_ids_invalid_strategy_returns_400() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_vector_collection(&app, "fusion_bad_ids").await;

    let resp = post(
        &app,
        "/collections/fusion_bad_ids/search/multi/ids",
        json!({
            "vectors": [[1.0, 0.0, 0.0, 0.0]],
            "top_k": 2,
            "strategy": "nope"
        }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// multi.rs — validate_query_vectors: wrong dimension reports the index
// ============================================================================

#[tokio::test]
async fn test_multi_query_search_wrong_dimension_reports_index() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    seed_vector_collection(&app, "dim_mismatch").await;

    // First vector matches dim=4; the second (index 1) is dim=2 -> 400.
    let resp = post(
        &app,
        "/collections/dim_mismatch/search/multi",
        json!({
            "vectors": [[1.0, 0.0, 0.0, 0.0], [1.0, 0.0]],
            "top_k": 2,
            "strategy": "rrf"
        }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = read_json(resp).await;
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("index 1"),
        "expected the offending vector index in the message, got {json:?}"
    );
}

// ============================================================================
// multi.rs — rate-limited preamble path (429) shared by both endpoints
// ============================================================================

#[tokio::test]
async fn test_multi_query_search_rate_limited_returns_429() {
    let temp_dir = TempDir::new().expect("temp dir");
    let (app, state) = create_test_app_with_state(&temp_dir);
    seed_vector_collection(&app, "multi_rl").await;

    // Exhaust the per-client token bucket for the default ("anonymous")
    // client so the next request trips `apply_pre_check` in
    // `prepare_multi_query`.
    let collection = state
        .db
        .get_vector_collection("multi_rl")
        .expect("collection exists after seeding");
    collection.guard_rails().rate_limiter.exhaust("anonymous");

    let resp = post(
        &app,
        "/collections/multi_rl/search/multi",
        json!({
            "vectors": [[1.0, 0.0, 0.0, 0.0]],
            "top_k": 2,
            "strategy": "rrf"
        }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

// ============================================================================
// search/mod.rs — the counters `search_request_pre_check` records for both
// `/search` and `/search/ids`
// ============================================================================

/// The vector-query counter, as `GET /metrics` names it.
const VECTOR_QUERIES: &str = "velesdb_queries_by_type{type=\"vector\"}";

/// The rate-limited query counter, as `GET /metrics` names it.
const RATE_LIMITED: &str = "velesdb_queries_total{status=\"rate_limited\"}";

/// Reads one counter out of a Prometheus exposition body: the line opening
/// with `key`, whose last whitespace-separated field is the value.
fn counter(exposition: &str, key: &str) -> u64 {
    let line = exposition
        .lines()
        .find(|line| line.starts_with(key))
        .unwrap_or_else(|| panic!("test: no `{key}` sample in:\n{exposition}"));
    line.split_whitespace()
        .next_back()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("test: `{line}` carries no numeric value"))
}

/// `/search` and `/search/ids` share `search_request_pre_check`, so both must
/// move the same two counters: one vector query per accepted request, and —
/// the counter being recorded BEFORE the guard rail, so a refused request
/// stays visible by type — one vector query AND one `status="rate_limited"`
/// on a 429.
///
/// The assertions read the very string `GET /metrics` serves for these two
/// families: `handlers/metrics.rs` pushes `operational_metrics
/// .export_prometheus()` into its body verbatim, and the integration router
/// in `tests/common/mod.rs` does not mount that route.
#[tokio::test]
async fn test_search_pre_check_records_vector_and_rate_limited_metrics() {
    for route in ["search", "search/ids"] {
        let temp_dir = TempDir::new().expect("temp dir");
        let (app, state) = create_test_app_with_state(&temp_dir);
        let name = "pre_check_metrics";
        seed_vector_collection(&app, name).await;
        let uri = format!("/collections/{name}/{route}");
        let body = json!({"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 2});

        let before = state.operational_metrics.export_prometheus();
        let resp = post(&app, &uri, body.clone()).await;
        assert_eq!(resp.status(), StatusCode::OK, "uri={uri}");
        let accepted = state.operational_metrics.export_prometheus();
        assert_eq!(
            counter(&accepted, VECTOR_QUERIES),
            counter(&before, VECTOR_QUERIES) + 1,
            "uri={uri}: an accepted search must count one vector query"
        );
        assert_eq!(
            counter(&accepted, RATE_LIMITED),
            counter(&before, RATE_LIMITED),
            "uri={uri}: an accepted search is not rate-limited"
        );

        // Exhaust the default ("anonymous") client's bucket so the next
        // request trips `apply_pre_check` inside the shared pre-check.
        state
            .db
            .get_vector_collection(name)
            .expect("collection exists after seeding")
            .guard_rails()
            .rate_limiter
            .exhaust("anonymous");

        let resp = post(&app, &uri, body).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS, "uri={uri}");
        let refused = state.operational_metrics.export_prometheus();
        assert_eq!(
            counter(&refused, RATE_LIMITED),
            counter(&accepted, RATE_LIMITED) + 1,
            "uri={uri}: a rate-limited search must count one rate-limited query"
        );
        assert_eq!(
            counter(&refused, VECTOR_QUERIES),
            counter(&accepted, VECTOR_QUERIES) + 1,
            "uri={uri}: the vector counter is recorded before the guard rail, \
             so a rate-limited search stays visible by type"
        );
    }
}

// ============================================================================
// graph/handlers.rs — build_edge rejects non-object / non-null properties
// ============================================================================

#[tokio::test]
async fn test_add_edge_invalid_properties_returns_400() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    create_graph_collection(&app, "edge_props").await;

    // `properties` is an array, which is neither an object nor null:
    // hits the `build_edge` 400 branch before any collection mutation.
    let resp = post(
        &app,
        "/collections/edge_props/graph/edges",
        json!({
            "id": 1,
            "source": 10,
            "target": 20,
            "label": "KNOWS",
            "properties": [1, 2, 3]
        }),
    )
    .await;

    assert_properties_shape_error(resp).await;
}

#[tokio::test]
async fn test_add_edges_batch_invalid_properties_returns_400() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    create_graph_collection(&app, "batch_props").await;

    // The batch path maps `build_edge` over every edge; a single edge with
    // a string `properties` value must fail the whole batch with a 400.
    let resp = post(
        &app,
        "/collections/batch_props/graph/edges/batch",
        json!({
            "edges": [
                {"id": 1, "source": 10, "target": 20, "label": "KNOWS", "properties": {}},
                {"id": 2, "source": 20, "target": 30, "label": "KNOWS", "properties": "oops"}
            ]
        }),
    )
    .await;

    assert_properties_shape_error(resp).await;
}

/// A `null` properties value is explicitly allowed and must succeed (201),
/// exercising the `Value::Null` arm of `build_edge`.
#[tokio::test]
async fn test_add_edge_null_properties_ok() {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);
    create_graph_collection(&app, "edge_null_props").await;
    create_graph_node(&app, "edge_null_props", 10).await;
    create_graph_node(&app, "edge_null_props", 20).await;

    let resp = post(
        &app,
        "/collections/edge_null_props/graph/edges",
        json!({
            "id": 1,
            "source": 10,
            "target": 20,
            "label": "KNOWS",
            "properties": null
        }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
}
