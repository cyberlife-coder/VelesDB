//! Route definitions for the VelesDB REST API.
//!
//! Centralises all Axum route registrations so they are shared between the
//! production binary (`main.rs`) and the test suite (OpenAPI conformance).

use std::sync::Arc;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post},
    Router,
};

use crate::{
    add_edge, add_edges_batch, aggregate, analyze_collection, batch_search, bulk_delete_points,
    collection_diagnostics, collection_sanity, compact_collection, create_collection, create_index,
    delete_collection, delete_index, delete_point, enable_streaming, explain, flush_collection,
    get_collection, get_collection_config, get_collection_stats, get_edge_count, get_edges,
    get_guardrails, get_node_degree, get_node_edges, get_node_payload, get_point,
    get_point_relations, graph_search, health_check, hybrid_search, is_empty, list_collections,
    list_indexes, list_nodes, match_query, multi_query_search, multi_query_search_ids, query,
    readiness_check, rebuild_index, relate_points, remove_edge, reorder_for_locality,
    scroll_points, search, search_ids, set_point_ttl, stream_insert, stream_traverse,
    stream_upsert_points, text_search, traverse_graph, traverse_parallel, unrelate_points,
    update_guardrails, upsert_node_payload, upsert_points, upsert_points_raw, vacuum_collection,
    AppState,
};

/// Default for `[server] max_body_size`: 100 MiB, the limit the point-upload
/// routes have always had (1000 vectors x 768D x 4 bytes is ~3 MB; 100 MiB
/// covers extreme batches).
pub const DEFAULT_MAX_BODY_SIZE: usize = 100 * 1024 * 1024;

/// Body limit of every route other than point upload: 2 MiB, axum's own
/// default, which those routes have always had. `[server] max_body_size` can
/// lower it, never raise it — a search or a query has no reason to carry a
/// body the size of a bulk upload.
pub const JSON_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Core CRUD and admin routes.
fn core_routes(max_body_size: usize) -> Router<Arc<AppState>> {
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
        .route(
            "/collections/{name}/diagnostics",
            get(collection_diagnostics),
        )
        .route("/guardrails", get(get_guardrails).put(update_guardrails))
        // `[server] max_body_size` in full applies to the batch upload routes
        // only. The NDJSON stream route reads its body incrementally and
        // never buffers it whole, so no body limit applies to it.
        .merge(
            Router::new()
                .route("/collections/{name}/points", post(upsert_points))
                .route("/collections/{name}/points/raw", post(upsert_points_raw))
                .route(
                    "/collections/{name}/points/stream",
                    post(stream_upsert_points),
                )
                .layer(DefaultBodyLimit::max(max_body_size)),
        )
        .route("/collections/{name}/stream/insert", post(stream_insert))
        .route("/collections/{name}/stream/enable", post(enable_streaming))
        .route(
            "/collections/{name}/points/{id}",
            get(get_point).delete(delete_point),
        )
        .route("/collections/{name}/points/scroll", post(scroll_points))
        // Bulk operations
        .route(
            "/collections/{name}/points/delete",
            post(bulk_delete_points),
        )
        // Maintenance endpoints
        .route("/collections/{name}/vacuum", post(vacuum_collection))
        .route("/collections/{name}/compact", post(compact_collection))
        .route(
            "/collections/{name}/locality/reorder",
            post(reorder_for_locality),
        )
}

/// Relation (graph edge) and durable TTL routes (any collection type).
fn relation_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/collections/{name}/relations", post(relate_points))
        .route(
            "/collections/{name}/relations/{edge_id}",
            delete(unrelate_points),
        )
        .route(
            "/collections/{name}/points/{id}/relations",
            get(get_point_relations),
        )
        .route("/collections/{name}/points/{id}/ttl", patch(set_point_ttl))
}

/// Search, text, hybrid, and index routes.
fn search_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/collections/{name}/search", post(search))
        .route("/collections/{name}/search/batch", post(batch_search))
        .route("/collections/{name}/search/multi", post(multi_query_search))
        .route(
            "/collections/{name}/search/multi/ids",
            post(multi_query_search_ids),
        )
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
            "/collections/{name}/graph/edges/batch",
            post(add_edges_batch),
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

/// All API routes merged into a single [`Router`], with the default body
/// limits: [`DEFAULT_MAX_BODY_SIZE`] on point upload, [`JSON_BODY_LIMIT`]
/// everywhere else.
///
/// This is the single source of truth for route registration. Both
/// the production binary and the OpenAPI conformance test consume it.
pub fn api_routes() -> Router<Arc<AppState>> {
    api_routes_with_body_limit(DEFAULT_MAX_BODY_SIZE)
}

/// [`api_routes`] with `max_body_size` (`[server] max_body_size`) as the
/// largest request body any route accepts. The point-upload routes
/// (`POST .../points`, `POST .../points/raw`) accept up to `max_body_size`;
/// every other route accepts up to the smaller of `max_body_size` and
/// [`JSON_BODY_LIMIT`]. A larger body is refused with `413 Payload Too
/// Large`. `POST .../points/stream` is not buffered and has no body limit.
pub fn api_routes_with_body_limit(max_body_size: usize) -> Router<Arc<AppState>> {
    let routes = core_routes(max_body_size)
        .merge(search_routes())
        .merge(graph_routes())
        .merge(relation_routes());
    #[cfg(feature = "prometheus")]
    let routes = routes.route("/metrics", get(crate::prometheus_metrics));
    // Outermost, so the upload routes' own `DefaultBodyLimit`, which runs
    // after it, overrides it for those routes alone.
    routes.layer(DefaultBodyLimit::max(max_body_size.min(JSON_BODY_LIMIT)))
}
