//! Route definitions for the VelesDB REST API.
//!
//! Centralises all Axum route registrations so they are shared between the
//! production binary (`main.rs`) and the test suite (OpenAPI conformance).

use std::sync::Arc;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};

use crate::{
    add_edge, aggregate, analyze_collection, batch_search, collection_sanity, create_collection,
    create_index, delete_collection, delete_index, delete_point, explain, flush_collection,
    get_collection, get_collection_config, get_collection_stats, get_edge_count, get_edges,
    get_guardrails, get_node_degree, get_node_edges, get_node_payload, get_point, graph_search,
    health_check, hybrid_search, is_empty, list_collections, list_indexes, list_nodes, match_query,
    multi_query_search, query, readiness_check, rebuild_index, remove_edge, scroll_points, search,
    search_ids, stream_insert, stream_traverse, stream_upsert_points, text_search, traverse_graph,
    traverse_parallel, update_guardrails, upsert_node_payload, upsert_points, AppState,
};

/// Core CRUD and admin routes.
fn core_routes() -> Router<Arc<AppState>> {
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
        .route("/collections/{name}/empty", get(is_empty))
        .route("/collections/{name}/config", get(get_collection_config))
        .route("/collections/{name}/sanity", get(collection_sanity))
        .route("/collections/{name}/flush", post(flush_collection))
        .route("/collections/{name}/analyze", post(analyze_collection))
        .route("/collections/{name}/index/rebuild", post(rebuild_index))
        .route("/collections/{name}/stats", get(get_collection_stats))
        .route("/guardrails", get(get_guardrails).put(update_guardrails))
        // 100 MB limit scoped to batch vector upload routes only
        // (1000 vectors x 768D x 4 bytes = ~3 MB typical; 100 MB covers extreme cases)
        .merge(
            Router::new()
                .route("/collections/{name}/points", post(upsert_points))
                .route(
                    "/collections/{name}/points/stream",
                    post(stream_upsert_points),
                )
                .layer(DefaultBodyLimit::max(100 * 1024 * 1024)),
        )
        .route("/collections/{name}/stream/insert", post(stream_insert))
        .route(
            "/collections/{name}/points/{id}",
            get(get_point).delete(delete_point),
        )
        .route("/collections/{name}/points/scroll", post(scroll_points))
}

/// Search, text, hybrid, and index routes.
fn search_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/collections/{name}/search", post(search))
        .route("/collections/{name}/search/batch", post(batch_search))
        .route("/collections/{name}/search/multi", post(multi_query_search))
        .route("/collections/{name}/search/text", post(text_search))
        .route("/collections/{name}/search/hybrid", post(hybrid_search))
        .route("/collections/{name}/search/ids", post(search_ids))
        .route(
            "/collections/{name}/indexes",
            get(list_indexes).post(create_index),
        )
        .route(
            "/collections/{name}/indexes/{label}/{property}",
            delete(delete_index),
        )
        .route("/query", post(query))
        .route("/aggregate", post(aggregate))
        .route("/query/explain", post(explain))
        .route("/collections/{name}/match", post(match_query))
}

/// Graph traversal and edge routes.
fn graph_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/collections/{name}/graph/edges",
            get(get_edges).post(add_edge),
        )
        .route(
            "/collections/{name}/graph/edges/{edge_id}",
            delete(remove_edge),
        )
        .route("/collections/{name}/graph/edges/count", get(get_edge_count))
        .route("/collections/{name}/graph/nodes", get(list_nodes))
        .route(
            "/collections/{name}/graph/nodes/{node_id}/edges",
            get(get_node_edges),
        )
        .route(
            "/collections/{name}/graph/nodes/{node_id}/payload",
            get(get_node_payload).put(upsert_node_payload),
        )
        .route(
            "/collections/{name}/graph/nodes/{node_id}/degree",
            get(get_node_degree),
        )
        .route("/collections/{name}/graph/traverse", post(traverse_graph))
        .route(
            "/collections/{name}/graph/traverse/parallel",
            post(traverse_parallel),
        )
        .route(
            "/collections/{name}/graph/traverse/stream",
            get(stream_traverse),
        )
        .route("/collections/{name}/graph/search", post(graph_search))
}

/// All API routes merged into a single [`Router`].
///
/// This is the single source of truth for route registration. Both
/// the production binary and the OpenAPI conformance test consume it.
pub fn api_routes() -> Router<Arc<AppState>> {
    core_routes().merge(search_routes()).merge(graph_routes())
}
