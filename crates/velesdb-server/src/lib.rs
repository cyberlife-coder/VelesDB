// Server — triaged pedantic/nursery lints (Sprint 2 Wave 8, A.10).
// Blanket `#![allow(clippy::pedantic)]` removed; each remaining lint is
// justified below.  Axum handler signatures, utoipa derives, and
// OpenAPI-documented error contracts drive most of these.
#![allow(clippy::uninlined_format_args)] // readability in error messages
#![allow(clippy::manual_let_else)] // pattern matching in handlers is clearer
#![allow(clippy::cast_possible_truncation)] // u128→u64 timing casts are bounded
#![allow(clippy::cast_sign_loss)] // Duration→u64 timing casts are non-negative
#![allow(clippy::cast_precision_loss)] // byte-count→f64 display casts are fine
#![allow(clippy::ref_option)] // utoipa-generated code triggers this
#![allow(clippy::match_same_arms)] // explicit arms improve readability in routers
#![allow(clippy::trivially_copy_pass_by_ref)] // Axum extractors require &
#![allow(clippy::map_unwrap_or)] // readability preference
#![allow(clippy::enum_glob_use)] // StatusCode::* in handlers
#![allow(clippy::unused_async)] // Axum requires async signature even for sync handlers
#![allow(clippy::needless_for_each)] // readability in metric recording loops
#![allow(clippy::doc_markdown)] // backtick pedantry — docs use utoipa annotations
#![allow(clippy::missing_errors_doc)] // errors documented in #[utoipa::path] responses
#![allow(clippy::must_use_candidate)] // handlers return impl IntoResponse, not Option
#![allow(clippy::similar_names)] // handler params are intentionally close (name/names)
#![allow(clippy::needless_raw_string_hashes)] // cosmetic, low-value fix
#![allow(clippy::needless_pass_by_value)] // Axum extractors consume by value
#![allow(clippy::redundant_closure_for_method_calls)] // readability in map chains
#![allow(clippy::single_match_else)] // pattern matching in handlers is clearer
#![allow(clippy::assigning_clones)]
// minor optimisation, not performance-critical
// The crate README is pulled into the crate documentation verbatim, so that
// `cargo test --doc --package velesdb-server` type-checks every ```rust block it
// contains. Today `README.md` only holds `bash`, `json` and `toml` blocks, which
// rustdoc never compiles; the include is what makes any future Rust snippet
// checked by the compiler instead of drifting away from the API unnoticed.
// Blocks that must not be compiled or executed have to carry an explicit
// rustdoc attribute in the README (`rust,no_run`, `rust,ignore`).
#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! # Crate-level notes
//!
//! `VelesDB` Server - REST API library for the `VelesDB` vector database.
//!
//! This module provides the HTTP handlers and types for the `VelesDB` REST API.
//!
//! ## OpenAPI Documentation
//!
//! The API is documented using OpenAPI 3.0. Access the interactive documentation at:
//! - Swagger UI: `GET /swagger-ui`
//! - OpenAPI JSON: `GET /api-docs/openapi.json`

pub mod auth;
pub mod config;
mod handlers;
pub mod onboarding;
pub mod rate_limit;
pub mod routes;
mod security_addon;
pub mod tls;
mod types;

use security_addon::SecurityAddon;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use utoipa::OpenApi;
use velesdb_core::{
    Database, DurationHistogram, OperationalMetrics, QueryLimits, TraversalMetrics,
};

pub use onboarding::OnboardingMetrics;
pub use types::*;

pub use handlers::{
    aggregate, analyze_collection, batch_search, bulk_delete_points, collection_diagnostics,
    collection_sanity, compact_collection, create_collection, create_index, delete_collection,
    delete_index, delete_point, enable_streaming, explain, flush_collection, get_collection,
    get_collection_config, get_collection_stats, get_guardrails, get_point, get_point_relations,
    health_check, hybrid_search, is_empty, list_collections, list_indexes, match_query,
    multi_query_search, multi_query_search_ids, query, readiness_check, rebuild_index,
    relate_points, reorder_for_locality, scroll_points, search, search_ids, set_point_ttl,
    stream_insert, stream_upsert_points, text_search, unrelate_points, update_guardrails,
    upsert_points, upsert_points_raw, vacuum_collection,
};

pub use handlers::graph::{
    add_edge, add_edges_batch, get_edge_count, get_edges, get_node_degree, get_node_edges,
    get_node_payload, graph_search, list_nodes, remove_edge, stream_traverse, traverse_graph,
    traverse_parallel, upsert_node_payload, DegreeResponse, EdgeCountResponse, GraphSearchRequest,
    GraphSearchResponse, NodeEdgeQueryParams, NodeListResponse, NodePayloadResponse,
    ParallelTraverseRequest, StreamDoneEvent, StreamNodeEvent, StreamStatsEvent,
    StreamTraverseParams, TraversalResultItem, TraversalStats, TraverseRequest, TraverseResponse,
    UpsertNodePayloadRequest,
};

#[cfg(feature = "prometheus")]
pub use handlers::metrics::{health_metrics, prometheus_metrics};

// ============================================================================
// OpenAPI Documentation

/// VelesDB API Documentation (paths that exist regardless of build features).
///
/// The `/metrics` path lives in [`MetricsApiDoc`] because `utoipa`'s `paths(...)`
/// list is a fixed macro argument list — individual entries can't carry a
/// `#[cfg(...)]`, so a handler gated behind the `prometheus` feature can't be
/// listed here unconditionally without breaking `--no-default-features` builds.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "VelesDB API",
        version = env!("CARGO_PKG_VERSION"),
        description = "High-performance vector database for AI applications. \
            Supports semantic search, HNSW indexing, and multiple distance metrics. \
            Authentication is optional — when API keys are configured via VELESDB_API_KEYS, \
            all endpoints except /health and /ready require a valid Bearer token.",
        license(name = "VelesDB Core License 1.0", url = "https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE"),
        contact(name = "VelesDB Team", url = "https://github.com/cyberlife-coder/VelesDB")
    ),
    security(
        ("bearer_auth" = [])
    ),
    modifiers(&SecurityAddon),
    servers(
        (url = "/", description = "Local server")
    ),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "collections", description = "Collection management"),
        (name = "points", description = "Vector point operations"),
        (name = "search", description = "Vector similarity search"),
        (name = "query", description = "VelesQL query execution"),
        (name = "indexes", description = "Property index management (EPIC-009)"),
        (name = "graph", description = "Graph traversal and edge operations"),
        (name = "guardrails", description = "Query guard-rails configuration (EPIC-048)"),
        (name = "metrics", description = "Prometheus operational metrics")
    ),
    paths(
        handlers::health::health_check,
        handlers::health::readiness_check,
        handlers::collections::list_collections,
        handlers::collections::create_collection,
        handlers::collections::get_collection,
        handlers::collections::delete_collection,
        handlers::collections::collection_sanity,
        handlers::collections::is_empty,
        handlers::collections::flush_collection,
        handlers::admin::analyze_collection,
        handlers::admin::get_collection_stats,
        handlers::admin::collection_diagnostics,
        handlers::admin::get_guardrails,
        handlers::admin::update_guardrails,
        handlers::points::upsert_points,
        handlers::points::raw::upsert_points_raw,
        handlers::points::stream_upsert_points,
        handlers::points::stream_insert,
        handlers::points::enable_streaming,
        handlers::points::get_point,
        handlers::points::delete_point,
        handlers::points::scroll_points,
        handlers::search::search,
        handlers::search::batch_search,
        handlers::search::multi_query_search,
        handlers::search::multi_query_search_ids,
        handlers::search::text_search,
        handlers::search::hybrid_search,
        handlers::search::search_ids,
        handlers::admin::get_collection_config,
        handlers::query::query,
        handlers::query::aggregate,
        handlers::query::explain,
        handlers::indexes::create_index,
        handlers::indexes::list_indexes,
        handlers::indexes::delete_index,
        handlers::graph::handlers::get_edges,
        handlers::graph::handlers::add_edge,
        handlers::graph::handlers::add_edges_batch,
        handlers::graph::handlers_extended::remove_edge,
        handlers::graph::handlers_extended::get_edge_count,
        handlers::graph::handlers_extended::list_nodes,
        handlers::graph::handlers_extended::get_node_edges,
        handlers::graph::handlers_extended::get_node_payload,
        handlers::graph::handlers_extended::upsert_node_payload,
        handlers::graph::handlers::traverse_graph,
        handlers::graph::handlers_extended::traverse_parallel,
        handlers::graph::handlers::get_node_degree,
        handlers::graph::handlers_extended::graph_search,
        handlers::graph::stream::stream_traverse,
        handlers::match_query::match_query,
        handlers::admin::rebuild_index,
        handlers::admin::vacuum_collection,
        handlers::admin::compact_collection,
        handlers::admin::reorder_for_locality,
        handlers::points::bulk_delete_points,
        handlers::points::relations::relate_points,
        handlers::points::relations::unrelate_points,
        handlers::points::relations::get_point_relations,
        handlers::points::relations::set_point_ttl,
    ),
    components(
        schemas(
            CreateCollectionRequest,
            CollectionResponse,
            UpsertPointsRequest,
            PointRequest,
            StreamInsertRequest,
            EnableStreamingRequest,
            SearchRequest,
            BatchSearchRequest,
            TextSearchRequest,
            HybridSearchRequest,
            MultiQuerySearchRequest,
            SearchResponse,
            BatchSearchResponse,
            SearchResultResponse,
            SearchIdsResponse,
            IdScoreResult,
            CollectionConfigResponse,
            ErrorResponse,
            QueryRequest,
            QueryResponse,
            QueryResponseMeta,
            AggregationResponse,
            QueryErrorResponse,
            QueryErrorDetail,
            VelesqlErrorResponse,
            VelesqlErrorDetail,
            ExplainRequest,
            ExplainResponse,
            ExplainStep,
            ExplainCost,
            ExplainFeatures,
            ActualStatsResponse,
            NodeStatsResponse,
            CreateIndexRequest,
            IndexResponse,
            ListIndexesResponse,
            CollectionStatsResponse,
            ColumnStatsResponse,
            IndexStatsResponse,
            ScrollRequest,
            ScrollResponse,
            ScrollPoint,
            GuardRailsConfigRequest,
            GuardRailsConfigResponse,
            CollectionDiagnosticsResponse,
            handlers::graph::TraverseRequest,
            handlers::graph::TraverseResponse,
            handlers::graph::TraversalResultItem,
            handlers::graph::TraversalStats,
            handlers::graph::DegreeResponse,
            handlers::graph::AddEdgeRequest,
            handlers::graph::AddEdgesBatchRequest,
            handlers::graph::AddEdgesBatchResponse,
            handlers::graph::EdgesResponse,
            handlers::graph::EdgeResponse,
            handlers::graph::EdgeCountResponse,
            handlers::graph::NodeListResponse,
            handlers::graph::NodePayloadResponse,
            handlers::graph::UpsertNodePayloadRequest,
            handlers::graph::ParallelTraverseRequest,
            handlers::graph::GraphSearchRequest,
            handlers::graph::GraphSearchResponse,
            handlers::graph::GraphSearchResultItem,
            handlers::graph::StreamNodeEvent,
            handlers::graph::StreamStatsEvent,
            handlers::graph::StreamDoneEvent,
            handlers::match_query::MatchQueryRequest,
            handlers::match_query::MatchQueryResponse,
            handlers::match_query::MatchQueryResultItem,
            handlers::match_query::MatchQueryMeta,
            handlers::points::BulkDeleteRequest,
            handlers::points::relations::RelateRequest,
            handlers::points::relations::RelateResponse,
            handlers::points::relations::RelationEdge,
            handlers::points::relations::RelationsResponse,
            handlers::points::relations::SetTtlRequest
        )
    )
)]
struct ApiDocBase;

/// OpenAPI doc fragment for the `/metrics` endpoint, only compiled when the
/// `prometheus` feature is enabled (see [`ApiDocBase`] for why this is split out).
#[cfg(feature = "prometheus")]
#[derive(OpenApi)]
#[openapi(paths(handlers::metrics::prometheus_metrics))]
struct MetricsApiDoc;

/// Public entry point for the full OpenAPI document. Merges in the
/// `prometheus`-gated `/metrics` path when that feature is enabled.
pub struct ApiDoc;

impl ApiDoc {
    pub fn openapi() -> utoipa::openapi::OpenApi {
        #[allow(unused_mut)]
        let mut doc = ApiDocBase::openapi();
        #[cfg(feature = "prometheus")]
        {
            doc = doc.merge_from(MetricsApiDoc::openapi());
        }
        doc
    }
}

// ============================================================================
// Application State

/// Application state shared across handlers.
pub struct AppState {
    /// The `VelesDB` database instance.
    pub db: Database,
    /// New-user onboarding diagnostics counters.
    pub onboarding_metrics: onboarding::OnboardingMetrics,
    /// Query guard-rails configuration (EPIC-048).
    pub query_limits: parking_lot::RwLock<QueryLimits>,
    /// Readiness flag — `true` once the database is fully loaded.
    pub ready: AtomicBool,
    /// Operational metrics: query throughput, connections, doc counts (EPIC-050).
    pub operational_metrics: Arc<OperationalMetrics>,
    /// Graph traversal metrics: nodes visited, depth, edges scanned.
    pub traversal_metrics: Arc<TraversalMetrics>,
    /// Query duration histogram for Prometheus export.
    pub query_duration_histogram: Arc<DurationHistogram>,
}

// ============================================================================
// Tests

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
