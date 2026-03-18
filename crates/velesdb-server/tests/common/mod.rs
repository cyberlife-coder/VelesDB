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
    get_collection, get_edges, get_node_degree, get_point, health_check, hybrid_search,
    list_collections, query, readiness_check, search, search_ids, stream_upsert_points,
    text_search, traverse_graph, upsert_points, AppState, OnboardingMetrics,
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
