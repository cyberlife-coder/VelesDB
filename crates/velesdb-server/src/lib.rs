#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::ref_option)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::trivially_copy_pass_by_ref)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::enum_glob_use)]
#![allow(clippy::unused_async)]
#![allow(clippy::needless_for_each)]
//! `VelesDB` Server - REST API library for the `VelesDB` vector database.
//!
//! This module provides the HTTP handlers and types for the `VelesDB` REST API.
//!
//! ## OpenAPI Documentation
//!
//! The API is documented using OpenAPI 3.0. Access the interactive documentation at:
//! - Swagger UI: `GET /swagger-ui`
//! - OpenAPI JSON: `GET /api-docs/openapi.json`

mod handlers;
mod types;

use utoipa::OpenApi;
use velesdb_core::Database;

// Re-export types for external use
pub use types::*;

// Re-export handlers for routing
pub use handlers::{
    batch_search, create_collection, create_index, delete_collection, delete_index, delete_point,
    get_collection, get_point, health_check, hybrid_search, list_collections, list_indexes, query,
    search, text_search, upsert_points,
};

// ============================================================================
// OpenAPI Documentation
// ============================================================================

/// VelesDB API Documentation
#[derive(OpenApi)]
#[openapi(
    info(
        title = "VelesDB API",
        version = "0.1.1",
        description = "High-performance vector database for AI applications. \
            Supports semantic search, HNSW indexing, and multiple distance metrics.",
        license(name = "ELv2", url = "https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE"),
        contact(name = "VelesDB Team", url = "https://github.com/cyberlife-coder/VelesDB")
    ),
    servers(
        (url = "/", description = "Local server")
    ),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "collections", description = "Collection management"),
        (name = "points", description = "Vector point operations"),
        (name = "search", description = "Vector similarity search"),
        (name = "query", description = "VelesQL query execution"),
        (name = "indexes", description = "Property index management (EPIC-009)")
    ),
    paths(
        handlers::health::health_check,
        handlers::collections::list_collections,
        handlers::collections::create_collection,
        handlers::collections::get_collection,
        handlers::collections::delete_collection,
        handlers::points::upsert_points,
        handlers::points::get_point,
        handlers::points::delete_point,
        handlers::search::search,
        handlers::search::batch_search,
        handlers::search::text_search,
        handlers::search::hybrid_search,
        handlers::query::query,
        handlers::indexes::create_index,
        handlers::indexes::list_indexes,
        handlers::indexes::delete_index
    ),
    components(
        schemas(
            CreateCollectionRequest,
            CollectionResponse,
            UpsertPointsRequest,
            PointRequest,
            SearchRequest,
            BatchSearchRequest,
            TextSearchRequest,
            HybridSearchRequest,
            SearchResponse,
            BatchSearchResponse,
            SearchResultResponse,
            ErrorResponse,
            QueryRequest,
            QueryResponse,
            QueryErrorResponse,
            QueryErrorDetail,
            CreateIndexRequest,
            IndexResponse,
            ListIndexesResponse
        )
    )
)]
pub struct ApiDoc;

// ============================================================================
// Application State
// ============================================================================

/// Application state shared across handlers.
pub struct AppState {
    /// The `VelesDB` database instance.
    pub db: Database,
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
        assert!(json.contains("0.1.1"), "Should contain version");
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
        assert!(json.contains("/search"), "Should document search endpoint");
        assert!(json.contains("/query"), "Should document /query");
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
    fn test_openapi_has_license() {
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
        assert!(json.contains("ELv2"), "Should have ELv2 license");
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
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\":\"Test error\""));
    }
}
