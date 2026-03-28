// Server - pedantic/nursery lints relaxed for Axum handler ergonomics
// and utoipa derive compatibility (many handlers are async for the Axum
// trait signature even when they don't await).
#![allow(clippy::pedantic)] // Axum handlers + utoipa derive generate pedantic warnings
#![allow(clippy::nursery)] // false positives on Axum extractors
#![allow(clippy::doc_markdown)] // utoipa doc attributes conflict with doc_markdown
#![allow(clippy::uninlined_format_args)] // readability in error messages
#![allow(clippy::manual_let_else)] // pattern matching in handlers is clearer
#![allow(clippy::cast_possible_truncation)] // u128→u64 timing casts are bounded
#![allow(clippy::ref_option)] // utoipa-generated code triggers this
#![allow(clippy::match_same_arms)] // explicit arms improve readability in routers
#![allow(clippy::trivially_copy_pass_by_ref)] // Axum extractors require &
#![allow(clippy::map_unwrap_or)] // readability preference
#![allow(clippy::enum_glob_use)] // StatusCode::* in handlers
#![allow(clippy::unused_async)] // Axum requires async signature even for sync handlers
#![allow(clippy::needless_for_each)] // readability in metric recording loops
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
pub mod tls;
mod types;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use utoipa::OpenApi;
use velesdb_core::guardrails::QueryLimits;
use velesdb_core::Database;

// Re-export types for external use
pub use types::*;

// Re-export handlers for routing
pub use handlers::{
    aggregate, analyze_collection, batch_search, collection_sanity, create_collection,
    create_index, delete_collection, delete_index, delete_point, explain, flush_collection,
    get_collection, get_collection_config, get_collection_stats, get_guardrails, get_point,
    health_check, hybrid_search, is_empty, list_collections, list_indexes, match_query,
    multi_query_search, query, readiness_check, search, search_ids, stream_insert,
    stream_upsert_points, text_search, update_guardrails, upsert_points,
};

// Graph handlers (EPIC-016/US-031)
pub use handlers::graph::{
    add_edge, get_edges, get_node_degree, stream_traverse, traverse_graph, DegreeResponse,
    StreamDoneEvent, StreamNodeEvent, StreamStatsEvent, StreamTraverseParams, TraversalResultItem,
    TraversalStats, TraverseRequest, TraverseResponse,
};

// FLAG-3 FIX: Re-export metrics handlers conditionally (EPIC-016/US-034,035)
#[cfg(feature = "prometheus")]
pub use handlers::metrics::{health_metrics, prometheus_metrics};

// ============================================================================
// OpenAPI Documentation
// ============================================================================

/// VelesDB API Documentation
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
        (name = "guardrails", description = "Query guard-rails configuration (EPIC-048)")
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
        handlers::admin::get_guardrails,
        handlers::admin::update_guardrails,
        handlers::points::upsert_points,
        handlers::points::stream_upsert_points,
        handlers::points::stream_insert,
        handlers::points::get_point,
        handlers::points::delete_point,
        handlers::search::search,
        handlers::search::batch_search,
        handlers::search::multi_query_search,
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
        handlers::graph::handlers::traverse_graph,
        handlers::graph::handlers::get_node_degree,
        handlers::graph::stream::stream_traverse,
        handlers::match_query::match_query
    ),
    components(
        schemas(
            CreateCollectionRequest,
            CollectionResponse,
            UpsertPointsRequest,
            PointRequest,
            StreamInsertRequest,
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
            CreateIndexRequest,
            IndexResponse,
            ListIndexesResponse,
            CollectionStatsResponse,
            ColumnStatsResponse,
            IndexStatsResponse,
            GuardRailsConfigRequest,
            GuardRailsConfigResponse,
            handlers::graph::TraverseRequest,
            handlers::graph::TraverseResponse,
            handlers::graph::TraversalResultItem,
            handlers::graph::TraversalStats,
            handlers::graph::DegreeResponse,
            handlers::graph::AddEdgeRequest,
            handlers::graph::EdgesResponse,
            handlers::graph::EdgeResponse,
            handlers::graph::StreamNodeEvent,
            handlers::graph::StreamStatsEvent,
            handlers::graph::StreamDoneEvent,
            handlers::match_query::MatchQueryRequest,
            handlers::match_query::MatchQueryResponse,
            handlers::match_query::MatchQueryResultItem,
            handlers::match_query::MatchQueryMeta,
            handlers::match_query::MatchQueryError
        )
    )
)]
pub struct ApiDoc;

/// Adds the Bearer authentication security scheme to the OpenAPI spec.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::Http::new(
                        utoipa::openapi::security::HttpAuthScheme::Bearer,
                    ),
                ),
            );
        }
    }
}

// ============================================================================
// Application State
// ============================================================================

/// Application state shared across handlers.
pub struct AppState {
    /// The `VelesDB` database instance.
    pub db: Database,
    /// New-user onboarding diagnostics counters.
    pub onboarding_metrics: OnboardingMetrics,
    /// Query guard-rails configuration (EPIC-048).
    pub query_limits: parking_lot::RwLock<QueryLimits>,
    /// Readiness flag — `true` once the database is fully loaded.
    pub ready: AtomicBool,
}

/// Lightweight counters for first-hour troubleshooting diagnostics.
#[derive(Default)]
pub struct OnboardingMetrics {
    pub search_requests_total: AtomicU64,
    pub dimension_mismatch_total: AtomicU64,
    pub empty_search_results_total: AtomicU64,
    pub filter_parse_errors_total: AtomicU64,
}

impl OnboardingMetrics {
    pub fn record_search_request(&self) {
        self.search_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_dimension_mismatch(&self) {
        self.dimension_mismatch_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_empty_search_results(&self) {
        self.empty_search_results_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_filter_parse_error(&self) {
        self.filter_parse_errors_total
            .fetch_add(1, Ordering::Relaxed);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use utoipa::OpenApi;

    #[test]
    fn test_openapi_spec_generation() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(!json.is_empty(), "OpenAPI spec should not be empty");
        assert!(json.contains("VelesDB API"), "Should contain API title");
        assert!(
            json.contains(env!("CARGO_PKG_VERSION")),
            "Should contain version"
        );
    }

    #[test]
    fn test_openapi_has_all_endpoints() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(json.contains("/health"), "Should document /health");
        assert!(
            json.contains("/collections"),
            "Should document /collections"
        );
        assert!(
            json.contains(r"/collections/{name}"),
            "Should document collections by name"
        );
        assert!(json.contains("/points"), "Should document points endpoint");
        assert!(
            json.contains(r"/collections/{name}/points/stream"),
            "Should document points stream endpoint"
        );
        assert!(json.contains("/search"), "Should document search endpoint");
        assert!(json.contains("/query"), "Should document /query");
        assert!(json.contains("/aggregate"), "Should document /aggregate");
        assert!(
            json.contains("/query/explain"),
            "Should document /query/explain"
        );
    }

    #[test]
    fn test_openapi_has_all_tags() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(json.contains("\"health\""), "Should have health tag");
        assert!(
            json.contains("\"collections\""),
            "Should have collections tag"
        );
        assert!(json.contains("\"points\""), "Should have points tag");
        assert!(json.contains("\"search\""), "Should have search tag");
        assert!(json.contains("\"query\""), "Should have query tag");
    }

    #[test]
    fn test_openapi_has_schemas() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(
            json.contains("CreateCollectionRequest"),
            "Should have CreateCollectionRequest schema"
        );
        assert!(
            json.contains("CollectionResponse"),
            "Should have CollectionResponse schema"
        );
        assert!(
            json.contains("SearchRequest"),
            "Should have SearchRequest schema"
        );
        assert!(
            json.contains("SearchResponse"),
            "Should have SearchResponse schema"
        );
        assert!(
            json.contains("ErrorResponse"),
            "Should have ErrorResponse schema"
        );
    }

    #[test]
    fn generate_openapi_spec_files() {
        let openapi = ApiDoc::openapi();
        let json = openapi
            .to_pretty_json()
            .expect("Failed to serialize OpenAPI JSON");
        let yaml = serde_yaml::to_string(&openapi).expect("Failed to serialize OpenAPI YAML");

        // Write to docs/ relative to workspace root
        let docs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("docs");
        std::fs::create_dir_all(&docs_dir).expect("Failed to create docs dir");

        std::fs::write(docs_dir.join("openapi.json"), &json).expect("Failed to write openapi.json");
        std::fs::write(docs_dir.join("openapi.yaml"), &yaml).expect("Failed to write openapi.yaml");

        // Verify key endpoints are present
        assert!(
            json.contains("sparse"),
            "OpenAPI spec should contain sparse endpoints"
        );
        assert!(
            json.contains("/graph/edges"),
            "Should contain graph edge endpoints"
        );
        assert!(
            json.contains("/graph/traverse"),
            "Should contain graph traverse endpoint"
        );
        assert!(
            json.contains("/stream/insert"),
            "Should contain stream insert endpoint"
        );
        assert!(
            json.contains("/match"),
            "Should contain match query endpoint"
        );
        assert!(
            json.contains("/search/multi"),
            "Should contain multi-query search endpoint"
        );
    }

    #[test]
    fn test_openapi_has_license() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(
            json.contains("VelesDB Core License 1.0"),
            "Should have VelesDB Core License 1.0"
        );
    }

    #[test]
    fn test_openapi_pretty_json() {
        let openapi = ApiDoc::openapi();
        let pretty_json = openapi
            .to_pretty_json()
            .expect("Failed to serialize pretty JSON");
        assert!(
            pretty_json.contains('\n'),
            "Pretty JSON should have newlines"
        );
        assert!(
            pretty_json.len() > 1000,
            "OpenAPI spec should be substantial"
        );
    }

    #[test]
    fn test_openapi_has_all_metrics_documented() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(json.contains("cosine"), "Should document cosine metric");
        assert!(
            json.contains("euclidean"),
            "Should document euclidean metric"
        );
        assert!(json.contains("dot"), "Should document dot product metric");
        assert!(json.contains("hamming"), "Should document hamming metric");
        assert!(json.contains("jaccard"), "Should document jaccard metric");
    }

    #[test]
    fn test_openapi_has_storage_mode_documented() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(
            json.contains("storage_mode"),
            "Should document storage_mode parameter"
        );
    }

    #[test]
    fn test_openapi_has_search_types_documented() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(json.contains("text_search"), "Should document text search");
        assert!(
            json.contains("hybrid_search"),
            "Should document hybrid search"
        );
        assert!(json.contains("batch"), "Should document batch search");
    }

    #[test]
    fn test_create_collection_request_default_metric() {
        let json = r#"{"name": "test", "dimension": 128}"#;
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.metric, "cosine");
    }

    #[test]
    fn test_create_collection_request_with_hamming() {
        let json = r#"{"name": "test", "dimension": 128, "metric": "hamming"}"#;
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.metric, "hamming");
    }

    #[test]
    fn test_create_collection_request_with_jaccard() {
        let json = r#"{"name": "test", "dimension": 128, "metric": "jaccard"}"#;
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.metric, "jaccard");
    }

    #[test]
    fn test_create_collection_request_with_storage_mode() {
        let json = r#"{"name": "test", "dimension": 128, "storage_mode": "sq8"}"#;
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.storage_mode, "sq8");
    }

    #[test]
    fn test_search_request_deserialize() {
        let json = r#"{"vector": [0.1, 0.2, 0.3], "top_k": 5}"#;
        let req: SearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.vector, vec![0.1, 0.2, 0.3]);
        assert_eq!(req.top_k, 5);
    }

    #[test]
    fn test_batch_search_request_deserialize() {
        let json = r#"{"searches": [{"vector": [0.1, 0.2], "top_k": 3}]}"#;
        let req: BatchSearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.searches.len(), 1);
        assert_eq!(req.searches[0].top_k, 3);
    }

    #[test]
    fn test_text_search_request_deserialize() {
        let json = r#"{"query": "machine learning", "top_k": 10}"#;
        let req: TextSearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query, "machine learning");
        assert_eq!(req.top_k, 10);
    }

    #[test]
    fn test_hybrid_search_request_deserialize() {
        let json = r#"{"vector": [0.1, 0.2], "query": "test", "top_k": 5}"#;
        let req: HybridSearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.vector, vec![0.1, 0.2]);
        assert_eq!(req.query, "test");
        assert_eq!(req.top_k, 5);
    }

    #[test]
    fn test_upsert_points_request_deserialize() {
        let json = r#"{"points": [{"id": 1, "vector": [0.1, 0.2]}]}"#;
        let req: UpsertPointsRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.points.len(), 1);
        assert_eq!(req.points[0].id, 1);
    }

    #[test]
    fn test_collection_response_serialize() {
        let resp = CollectionResponse {
            name: "test".to_string(),
            dimension: 128,
            metric: "cosine".to_string(),
            storage_mode: "full".to_string(),
            point_count: 100,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"dimension\":128"));
        assert!(json.contains("\"metric\":\"cosine\""));
        assert!(json.contains("\"storage_mode\":\"full\""));
        assert!(json.contains("\"point_count\":100"));
    }

    #[test]
    fn test_search_response_serialize() {
        let resp = SearchResponse {
            results: vec![SearchResultResponse {
                id: 1,
                score: 0.95,
                payload: None,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"results\""));
        assert!(json.contains("\"id\":1"));
    }

    #[test]
    fn test_error_response_serialize() {
        let resp = ErrorResponse {
            error: "Test error".to_string(),
            code: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\":\"Test error\""));
        // code: None is omitted from JSON output
        assert!(!json.contains("\"code\""));
    }
}
