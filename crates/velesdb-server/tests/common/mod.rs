//! Common test utilities for velesdb-server integration tests.
#![allow(dead_code)]

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tempfile::TempDir;

use velesdb_core::Database;
use velesdb_server::{
    add_edge, aggregate,
    auth::{auth_middleware, AuthState},
    batch_search, collection_sanity, create_collection, delete_collection, delete_point, explain,
    get_collection, get_collection_config, get_edges, get_node_degree, get_point, health_check,
    hybrid_search, list_collections, multi_query_search, query, readiness_check, rebuild_index,
    search, search_ids, stream_upsert_points, text_search, traverse_graph, upsert_points,
    AppState, OnboardingMetrics,
};

fn base_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .route(
            "/collections",
            get(list_collections).post(create_collection),
        )
        .route(
            "/collections/{name}",
            get(get_collection).delete(delete_collection),
        )
        .route("/collections/{name}/config", get(get_collection_config))
        .route("/collections/{name}/index/rebuild", post(rebuild_index))
        .route("/collections/{name}/sanity", get(collection_sanity))
        .route("/collections/{name}/points", post(upsert_points))
        .route(
            "/collections/{name}/points/stream",
            post(stream_upsert_points),
        )
        .route(
            "/collections/{name}/points/{id}",
            get(get_point).delete(delete_point),
        )
        .route("/collections/{name}/search", post(search))
        .route("/collections/{name}/search/batch", post(batch_search))
        .route("/collections/{name}/search/multi", post(multi_query_search))
        .route("/collections/{name}/search/text", post(text_search))
        .route("/collections/{name}/search/hybrid", post(hybrid_search))
        .route("/collections/{name}/search/ids", post(search_ids))
        .route("/query", post(query))
        .route("/aggregate", post(aggregate))
        .route("/query/explain", post(explain))
        .route(
            "/collections/{name}/graph/edges",
            get(get_edges).post(add_edge),
        )
        .route("/collections/{name}/graph/traverse", post(traverse_graph))
        .route(
            "/collections/{name}/graph/nodes/{node_id}/degree",
            get(get_node_degree),
        )
}

fn create_app_state(temp_dir: &TempDir) -> Arc<AppState> {
    let db = Database::open(temp_dir.path()).expect("Failed to open database");
    Arc::new(AppState {
        db,
        onboarding_metrics: OnboardingMetrics::default(),
        query_limits: parking_lot::RwLock::new(velesdb_core::guardrails::QueryLimits::default()),
        ready: std::sync::atomic::AtomicBool::new(true),
    })
}

/// Helper to create test app with all routes (no auth).
pub fn create_test_app(temp_dir: &TempDir) -> Router {
    base_routes().with_state(create_app_state(temp_dir))
}

/// Helper to create test app and return the shared state for direct manipulation.
pub fn create_test_app_with_state(temp_dir: &TempDir) -> (Router, Arc<AppState>) {
    let state = create_app_state(temp_dir);
    let router = base_routes().with_state(Arc::clone(&state));
    (router, state)
}

/// Helper to create test app with API key authentication enabled.
pub fn create_test_app_with_auth(temp_dir: &TempDir, api_keys: Vec<String>) -> Router {
    let state = create_app_state(temp_dir);
    let auth_state = AuthState::new(api_keys);
    base_routes()
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            auth_middleware,
        ))
}

/// Middleware that adds deprecation headers for unversioned legacy routes.
/// Mirrors the production middleware in `main.rs`.
async fn deprecation_header(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "deprecation",
        "true".parse().expect("test: static header value"),
    );
    headers.insert(
        "x-api-deprecated",
        "Use /v1/ prefix"
            .parse()
            .expect("test: static header value"),
    );
    response
}

/// Helper to create test app with `/v1/` versioned routes and legacy
/// unversioned routes (with deprecation headers). Mirrors `build_router()`
/// from the production binary.
pub fn create_versioned_test_app(temp_dir: &TempDir) -> Router {
    let state = create_app_state(temp_dir);
    let routes = base_routes();

    // Canonical versioned API under /v1/
    let versioned = Router::new().nest("/v1", routes.clone());

    // Legacy unversioned routes with deprecation headers
    let legacy = routes.layer(axum::middleware::from_fn(deprecation_header));

    versioned.merge(legacy).with_state(state)
}

/// Seeds a graph collection via `POST /collections` with
/// `collection_type = "graph"`. Returns after asserting the 201 status.
///
/// Since F-05 (Sprint 1), graph collections must be created explicitly
/// before any `/collections/{name}/graph/*` endpoint can be called.
/// Previously, `get_graph_collection_or_404` auto-created a schemaless
/// graph collection on first use, which made tests appear to work
/// without an explicit creation step but hid a feature-lie from
/// real API consumers.
pub async fn create_graph_collection(app: &Router, name: &str) {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "name": name,
                        "collection_type": "graph"
                    })
                    .to_string(),
                ))
                .expect("test: build create graph collection request"),
        )
        .await
        .expect("test: create graph collection request failed");

    assert_eq!(
        response.status(),
        axum::http::StatusCode::CREATED,
        "test: failed to create graph collection '{name}'"
    );
}
