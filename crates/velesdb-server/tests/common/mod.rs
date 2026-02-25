//! Common test utilities for velesdb-server integration tests.

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::oneshot;

use velesdb_core::Database;
use velesdb_server::{
    add_edge, batch_search, create_collection, delete_collection, delete_point, get_collection,
    get_edges, get_node_degree, get_point, health_check, hybrid_search, list_collections, query,
    search, stream_traverse, text_search, traverse_graph, upsert_points, AppState, GraphService,
};

/// Helper to create test app with all routes
pub fn create_test_app(temp_dir: &TempDir) -> Router {
    let db = Database::open(temp_dir.path()).expect("Failed to open database");
    let state = Arc::new(AppState { db });
    let graph_service = GraphService::new();

    Router::new()
        .route("/health", get(health_check))
        .route(
            "/collections",
            get(list_collections).post(create_collection),
        )
        .route(
            "/collections/{name}",
            get(get_collection).delete(delete_collection),
        )
        .route("/collections/{name}/points", post(upsert_points))
        .route(
            "/collections/{name}/points/{id}",
            get(get_point).delete(delete_point),
        )
        .route("/collections/{name}/search", post(search))
        .route("/collections/{name}/search/batch", post(batch_search))
        .route("/collections/{name}/search/text", post(text_search))
        .route("/collections/{name}/search/hybrid", post(hybrid_search))
        .route("/query", post(query))
        .with_state(state)
        .route(
            "/collections/{name}/graph/edges",
            get(get_edges).post(add_edge),
        )
        .route("/collections/{name}/graph/traverse", post(traverse_graph))
        .route(
            "/collections/{name}/graph/traverse/stream",
            get(stream_traverse),
        )
        .route(
            "/collections/{name}/graph/nodes/{node_id}/degree",
            get(get_node_degree),
        )
        .with_state(graph_service)
}

/// Spawn a test HTTP server on an ephemeral port and return its base URL.
#[allow(dead_code)]
pub async fn spawn_test_server(temp_dir: &TempDir) -> (String, oneshot::Sender<()>) {
    let app = create_test_app(temp_dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test listener");
    let addr = listener
        .local_addr()
        .expect("Failed to read local listener addr");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("Test server failed");
    });

    (format!("http://{addr}"), shutdown_tx)
}
