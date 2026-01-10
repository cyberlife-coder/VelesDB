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
#![allow(clippy::needless_for_each)] // Required for utoipa OpenApi derive macro
//! `VelesDB` Server - REST API library for the `VelesDB` vector database.
//!
//! This module provides the HTTP handlers and types for the `VelesDB` REST API.
//!
//! ## OpenAPI Documentation
//!
//! The API is documented using OpenAPI 3.0. Access the interactive documentation at:
//! - Swagger UI: `GET /swagger-ui`
//! - OpenAPI JSON: `GET /api-docs/openapi.json`

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{OpenApi, ToSchema};

use velesdb_core::velesql;
use velesdb_core::{Database, DistanceMetric, Point, StorageMode};

// ============================================================================
// OpenAPI Documentation
// ============================================================================

/// VelesDB API Documentation
///
/// High-performance vector database for AI applications.
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
        (name = "query", description = "VelesQL query execution")
    ),
    paths(
        health_check,
        list_collections,
        create_collection,
        get_collection,
        delete_collection,
        upsert_points,
        get_point,
        delete_point,
        search,
        batch_search,
        text_search,
        hybrid_search,
        query
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
            QueryErrorDetail
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
// Request/Response Types
// ============================================================================

/// Request to create a new collection.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCollectionRequest {
    /// Collection name.
    #[schema(example = "documents")]
    pub name: String,
    /// Vector dimension (required for vector collections, ignored for metadata_only).
    #[schema(example = 768)]
    pub dimension: Option<usize>,
    /// Distance metric (cosine, euclidean, dot, hamming, jaccard).
    #[serde(default = "default_metric")]
    #[schema(example = "cosine")]
    pub metric: String,
    /// Storage mode (full, sq8, binary). Defaults to full.
    #[serde(default = "default_storage_mode")]
    #[schema(example = "full")]
    pub storage_mode: String,
    /// Collection type: "vector" (default) or "metadata_only".
    #[serde(default = "default_collection_type")]
    #[schema(example = "vector")]
    pub collection_type: String,
}

fn default_collection_type() -> String {
    "vector".to_string()
}

fn default_metric() -> String {
    "cosine".to_string()
}

fn default_storage_mode() -> String {
    "full".to_string()
}

/// Response with collection information.
#[derive(Debug, Serialize, ToSchema)]
pub struct CollectionResponse {
    /// Collection name.
    pub name: String,
    /// Vector dimension.
    pub dimension: usize,
    /// Distance metric.
    pub metric: String,
    /// Number of points in the collection.
    pub point_count: usize,
    /// Storage mode (full, sq8, binary).
    pub storage_mode: String,
}

/// Request to upsert points.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertPointsRequest {
    /// Points to upsert.
    pub points: Vec<PointRequest>,
}

/// A point in an upsert request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PointRequest {
    /// Point ID.
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Optional payload.
    pub payload: Option<serde_json::Value>,
}

/// Request for vector search.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchRequest {
    /// Query vector.
    pub vector: Vec<f32>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Search mode preset: fast, balanced, accurate, perfect.
    /// Overrides ef_search with predefined values.
    #[serde(default)]
    #[schema(example = "balanced")]
    pub mode: Option<String>,
    /// HNSW ef_search parameter (higher = better recall, slower).
    /// Overrides mode if both are specified.
    #[serde(default)]
    #[schema(example = 128)]
    pub ef_search: Option<usize>,
    /// Query timeout in milliseconds.
    #[serde(default)]
    #[schema(example = 30000)]
    pub timeout_ms: Option<u64>,
    /// Optional metadata filter to apply to results (JSON object with condition).
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for batch vector search.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchSearchRequest {
    /// List of search requests.
    pub searches: Vec<SearchRequest>,
}

fn default_top_k() -> usize {
    10
}

/// Convert mode string to ef_search value.
fn mode_to_ef_search(mode: &str) -> Option<usize> {
    match mode.to_lowercase().as_str() {
        "fast" => Some(64),
        "balanced" => Some(128),
        "accurate" => Some(256),
        "perfect" => Some(usize::MAX), // Will trigger brute-force
        _ => None,
    }
}

/// Response from vector search.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    /// Search results.
    pub results: Vec<SearchResultResponse>,
}

/// Response from batch search.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchSearchResponse {
    /// Results for each search query.
    pub results: Vec<SearchResponse>,
    /// Total time in milliseconds.
    pub timing_ms: f64,
}

/// A single search result.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResultResponse {
    /// Point ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Point payload.
    pub payload: Option<serde_json::Value>,
}

/// Error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Error message.
    pub error: String,
}

/// Request for BM25 text search.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TextSearchRequest {
    /// Text query for full-text search.
    #[schema(example = "rust programming")]
    pub query: String,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[schema(example = 10)]
    pub top_k: usize,
    /// Optional metadata filter to apply to results (JSON object with condition).
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for hybrid search (vector + text).
#[derive(Debug, Deserialize, ToSchema)]
pub struct HybridSearchRequest {
    /// Query vector for similarity search.
    pub vector: Vec<f32>,
    /// Text query for BM25 search.
    #[schema(example = "rust programming")]
    pub query: String,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[schema(example = 10)]
    pub top_k: usize,
    /// Weight for vector similarity (0.0-1.0). Text weight = 1 - vector_weight.
    #[serde(default = "default_vector_weight")]
    #[schema(example = 0.5)]
    pub vector_weight: f32,
    /// Optional metadata filter to apply to results (JSON object with condition).
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

fn default_vector_weight() -> f32 {
    0.5
}

/// Request for multi-query vector search with fusion.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MultiQuerySearchRequest {
    /// List of query vectors.
    pub vectors: Vec<Vec<f32>>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[schema(example = 10)]
    pub top_k: usize,
    /// Fusion strategy: "average", "maximum", "rrf", "weighted".
    #[serde(default = "default_fusion_strategy")]
    #[schema(example = "rrf")]
    pub strategy: String,
    /// RRF k parameter (only used when strategy = "rrf").
    #[serde(default = "default_rrf_k")]
    #[schema(example = 60)]
    pub rrf_k: u32,
    /// Optional metadata filter to apply to results.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

fn default_fusion_strategy() -> String {
    "rrf".to_string()
}

fn default_rrf_k() -> u32 {
    60
}

/// Request for `VelesQL` query execution.
#[derive(Debug, Deserialize, ToSchema)]
pub struct QueryRequest {
    /// The `VelesQL` query string.
    pub query: String,
    /// Named parameters for the query.
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
}

/// Response from VelesQL query execution.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueryResponse {
    /// Query results.
    pub results: Vec<SearchResultResponse>,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
    /// Number of rows returned.
    pub rows_returned: usize,
}

/// VelesQL query error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueryErrorResponse {
    /// Error details.
    pub error: QueryErrorDetail,
}

/// VelesQL query error detail.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueryErrorDetail {
    /// Error type.
    #[serde(rename = "type")]
    pub error_type: String,
    /// Error message.
    pub message: String,
    /// Position in query where error occurred.
    pub position: usize,
    /// Fragment of query around error.
    pub query: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// Health check endpoint.
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Server is healthy", body = Object)
    )
)]
pub async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// List all collections.
#[utoipa::path(
    get,
    path = "/collections",
    tag = "collections",
    responses(
        (status = 200, description = "List of collections", body = Object)
    )
)]
pub async fn list_collections(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let collections = state.db.list_collections();
    Json(serde_json::json!({ "collections": collections }))
}

/// Create a new collection.
#[utoipa::path(
    post,
    path = "/collections",
    tag = "collections",
    request_body = CreateCollectionRequest,
    responses(
        (status = 201, description = "Collection created", body = Object),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
pub async fn create_collection(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCollectionRequest>,
) -> impl IntoResponse {
    let metric = match req.metric.to_lowercase().as_str() {
        "cosine" => DistanceMetric::Cosine,
        "euclidean" | "l2" => DistanceMetric::Euclidean,
        "dot" | "dotproduct" | "ip" => DistanceMetric::DotProduct,
        "hamming" => DistanceMetric::Hamming,
        "jaccard" => DistanceMetric::Jaccard,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid metric: {}. Valid: cosine, euclidean, dot, hamming, jaccard",
                        req.metric
                    ),
                }),
            )
                .into_response()
        }
    };

    let storage_mode = match req.storage_mode.to_lowercase().as_str() {
        "full" | "f32" => StorageMode::Full,
        "sq8" | "int8" => StorageMode::SQ8,
        "binary" | "bit" => StorageMode::Binary,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid storage_mode: {}. Valid: full, sq8, binary",
                        req.storage_mode
                    ),
                }),
            )
                .into_response()
        }
    };

    // Handle collection type
    let result = match req.collection_type.to_lowercase().as_str() {
        "metadata_only" | "metadata-only" => {
            use velesdb_core::CollectionType;
            state
                .db
                .create_collection_typed(&req.name, &CollectionType::MetadataOnly)
        }
        "vector" | "" => {
            let dimension = match req.dimension {
                Some(d) => d,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "dimension is required for vector collections".to_string(),
                        }),
                    )
                        .into_response()
                }
            };
            state
                .db
                .create_collection_with_options(&req.name, dimension, metric, storage_mode)
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid collection_type: {}. Valid: vector, metadata_only",
                        req.collection_type
                    ),
                }),
            )
                .into_response()
        }
    };

    match result {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "message": "Collection created",
                "name": req.name,
                "type": req.collection_type
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Get collection information.
#[utoipa::path(
    get,
    path = "/collections/{name}",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection details", body = CollectionResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn get_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.get_collection(&name) {
        Some(collection) => {
            let config = collection.config();
            Json(CollectionResponse {
                name: config.name,
                dimension: config.dimension,
                metric: format!("{:?}", config.metric).to_lowercase(),
                point_count: config.point_count,
                storage_mode: format!("{:?}", config.storage_mode).to_lowercase(),
            })
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{}' not found", name),
            }),
        )
            .into_response(),
    }
}

/// Delete a collection.
#[utoipa::path(
    delete,
    path = "/collections/{name}",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection deleted", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn delete_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.delete_collection(&name) {
        Ok(()) => Json(serde_json::json!({
            "message": "Collection deleted",
            "name": name
        }))
        .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Upsert points into a collection.
#[utoipa::path(
    post,
    path = "/collections/{name}/points",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = UpsertPointsRequest,
    responses(
        (status = 200, description = "Points upserted", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
pub async fn upsert_points(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<UpsertPointsRequest>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    let points: Vec<Point> = req
        .points
        .into_iter()
        .map(|p| Point::new(p.id, p.vector, p.payload))
        .collect();

    // CRITICAL: upsert_bulk is blocking (HNSW insertion + I/O)
    // Must use spawn_blocking to avoid blocking the async runtime
    let result = tokio::task::spawn_blocking(move || collection.upsert_bulk(&points)).await;

    match result {
        Ok(Ok(inserted)) => Json(serde_json::json!({
            "message": "Points upserted",
            "count": inserted
        }))
        .into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Task panicked: {e}"),
            }),
        )
            .into_response(),
    }
}

/// Get a point by ID.
#[utoipa::path(
    get,
    path = "/collections/{name}/points/{id}",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("id" = u64, Path, description = "Point ID")
    ),
    responses(
        (status = 200, description = "Point found", body = Object),
        (status = 404, description = "Point or collection not found", body = ErrorResponse)
    )
)]
pub async fn get_point(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, u64)>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    let points = collection.get(&[id]);

    match points.into_iter().next().flatten() {
        Some(point) => Json(serde_json::json!({
            "id": point.id,
            "vector": point.vector,
            "payload": point.payload
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Point {} not found", id),
            }),
        )
            .into_response(),
    }
}

/// Delete a point by ID.
#[utoipa::path(
    delete,
    path = "/collections/{name}/points/{id}",
    tag = "points",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("id" = u64, Path, description = "Point ID")
    ),
    responses(
        (status = 200, description = "Point deleted", body = Object),
        (status = 404, description = "Point or collection not found", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)] // Axum handler requires async
pub async fn delete_point(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, u64)>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    match collection.delete(&[id]) {
        Ok(()) => Json(serde_json::json!({
            "message": "Point deleted",
            "id": id
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Search for similar vectors.
#[utoipa::path(
    post,
    path = "/collections/{name}/search",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = SearchRequest,
    responses(
        (status = 200, description = "Search results", body = SearchResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)] // Axum handler requires async
pub async fn search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    // Determine effective ef_search from mode or explicit value
    let effective_ef = req
        .ef_search
        .or_else(|| req.mode.as_ref().and_then(|m| mode_to_ef_search(m)));

    // Use filtered search if filter is provided
    let search_result = if let Some(ref filter_json) = req.filter {
        // Deserialize JSON to Filter
        let filter: velesdb_core::Filter = match serde_json::from_value(filter_json.clone()) {
            Ok(f) => f,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response()
            }
        };
        collection.search_with_filter(&req.vector, req.top_k, &filter)
    } else if let Some(ef) = effective_ef {
        collection.search_with_ef(&req.vector, req.top_k, ef)
    } else {
        collection.search(&req.vector, req.top_k)
    };

    match search_result {
        Ok(results) => {
            let response = SearchResponse {
                results: results
                    .into_iter()
                    .map(|r| SearchResultResponse {
                        id: r.point.id,
                        score: r.score,
                        payload: r.point.payload,
                    })
                    .collect(),
            };
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Batch search for multiple vectors.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/batch",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = BatchSearchRequest,
    responses(
        (status = 200, description = "Batch search results", body = BatchSearchResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)] // Axum handler requires async
pub async fn batch_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<BatchSearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    // Perf P0: Use parallel batch search instead of sequential loop
    // Collect query vectors and optional filters
    let queries: Vec<&[f32]> = req.searches.iter().map(|s| s.vector.as_slice()).collect();

    let filters: Vec<Option<velesdb_core::Filter>> = req
        .searches
        .iter()
        .map(|s| {
            s.filter
                .as_ref()
                .and_then(|f_json| serde_json::from_value(f_json.clone()).ok())
        })
        .collect();

    // Assume all searches have the same top_k (use first or default)
    let top_k = req.searches.first().map_or(10, |s| s.top_k);

    let all_results = match collection.search_batch_with_filters(&queries, top_k, &filters) {
        Ok(batch_results) => batch_results
            .into_iter()
            .map(|results| SearchResponse {
                results: results
                    .into_iter()
                    .map(|r| SearchResultResponse {
                        id: r.point.id,
                        score: r.score,
                        payload: r.point.payload,
                    })
                    .collect(),
            })
            .collect(),
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;

    Json(BatchSearchResponse {
        results: all_results,
        timing_ms,
    })
    .into_response()
}

/// Multi-query search with fusion strategies.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/multi",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = MultiQuerySearchRequest,
    responses(
        (status = 200, description = "Multi-query fusion search results", body = SearchResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn multi_query_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<MultiQuerySearchRequest>,
) -> impl IntoResponse {
    use velesdb_core::FusionStrategy;

    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    // Parse fusion strategy
    let strategy = match req.strategy.to_lowercase().as_str() {
        "average" | "avg" => FusionStrategy::Average,
        "maximum" | "max" => FusionStrategy::Maximum,
        "rrf" => FusionStrategy::RRF { k: req.rrf_k },
        "weighted" => FusionStrategy::Weighted {
            avg_weight: 0.5,
            max_weight: 0.3,
            hit_weight: 0.2,
        },
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid strategy: {}. Valid: average, maximum, rrf, weighted",
                        req.strategy
                    ),
                }),
            )
                .into_response()
        }
    };

    // Convert vectors to slices
    let query_refs: Vec<&[f32]> = req.vectors.iter().map(|v| v.as_slice()).collect();

    // Execute multi-query search (4th arg: optional filter)
    let results = match collection.multi_query_search(&query_refs, req.top_k, strategy, None) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    };

    let response = SearchResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
    };

    Json(response).into_response()
}

/// Search using BM25 full-text search.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/text",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = TextSearchRequest,
    responses(
        (status = 200, description = "Text search results", body = SearchResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn text_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<TextSearchRequest>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    // Use filtered search if filter is provided
    let results = if let Some(ref filter_json) = req.filter {
        // Deserialize JSON to Filter
        let filter: velesdb_core::Filter = match serde_json::from_value(filter_json.clone()) {
            Ok(f) => f,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response()
            }
        };
        collection.text_search_with_filter(&req.query, req.top_k, &filter)
    } else {
        collection.text_search(&req.query, req.top_k)
    };

    let response = SearchResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
    };

    Json(response).into_response()
}

/// Hybrid search combining vector similarity and BM25 text search.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/hybrid",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = HybridSearchRequest,
    responses(
        (status = 200, description = "Hybrid search results", body = SearchResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn hybrid_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<HybridSearchRequest>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    // Use filtered search if filter is provided
    let search_result = if let Some(ref filter_json) = req.filter {
        // Deserialize JSON to Filter
        let filter: velesdb_core::Filter = match serde_json::from_value(filter_json.clone()) {
            Ok(f) => f,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response()
            }
        };
        collection.hybrid_search_with_filter(
            &req.vector,
            &req.query,
            req.top_k,
            Some(req.vector_weight),
            &filter,
        )
    } else {
        collection.hybrid_search(&req.vector, &req.query, req.top_k, Some(req.vector_weight))
    };

    match search_result {
        Ok(results) => {
            let response = SearchResponse {
                results: results
                    .into_iter()
                    .map(|r| SearchResultResponse {
                        id: r.point.id,
                        score: r.score,
                        payload: r.point.payload,
                    })
                    .collect(),
            };
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Execute a VelesQL query.
///
/// POST /query
/// ```json
/// {
///   "query": "SELECT * FROM documents WHERE vector NEAR $v LIMIT 10",
///   "params": { "v": [0.1, 0.2, ...] }
/// }
/// ```
#[utoipa::path(
    post,
    path = "/query",
    tag = "query",
    request_body = QueryRequest,
    responses(
        (status = 200, description = "Query results", body = QueryResponse),
        (status = 400, description = "Query syntax error", body = QueryErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    // Parse the VelesQL query
    let parsed = match velesql::Parser::parse(&req.query) {
        Ok(q) => q,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(QueryErrorResponse {
                    error: QueryErrorDetail {
                        error_type: format!("{:?}", e.kind),
                        message: e.message.clone(),
                        position: e.position,
                        query: e.fragment.clone(),
                    },
                }),
            )
                .into_response()
        }
    };

    let select = &parsed.select;

    // Get collection
    let collection = match state.db.get_collection(&select.from) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", select.from),
                }),
            )
                .into_response()
        }
    };

    // Use unified execute_query method from Collection
    let results = match collection.execute_query(&parsed, &req.params) {
        Ok(r) => r,
        Err(e) => return bad_request_response(&e.to_string()),
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    let rows_returned = results.len();

    Json(QueryResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
        timing_ms,
        rows_returned,
    })
    .into_response()
}

/// Helper to create a BAD_REQUEST error response.
fn bad_request_response(error: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
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
        // Arrange & Act
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");

        // Assert
        assert!(!json.is_empty(), "OpenAPI spec should not be empty");
        assert!(json.contains("VelesDB API"), "Should contain API title");
        assert!(json.contains("0.1.1"), "Should contain version");
    }

    #[test]
    fn test_openapi_has_all_endpoints() {
        // Arrange
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");

        // Assert - all endpoints are documented
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
        // Arrange
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");

        // Assert - all tags are present
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
        // Arrange
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");

        // Assert - schemas are defined
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
        // Arrange
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");

        // Assert - license is specified
        assert!(json.contains("ELv2"), "Should have ELv2 license");
    }

    #[test]
    fn test_openapi_pretty_json() {
        // Arrange
        let openapi = ApiDoc::openapi();

        // Act
        let pretty_json = openapi
            .to_pretty_json()
            .expect("Failed to serialize pretty JSON");

        // Assert
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
        // Arrange
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");

        // Assert - all distance metrics documented
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
        // Arrange
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");

        // Assert - storage mode is documented
        assert!(
            json.contains("storage_mode"),
            "Should document storage_mode parameter"
        );
    }

    #[test]
    fn test_openapi_has_search_types_documented() {
        // Arrange
        let openapi = ApiDoc::openapi();
        let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");

        // Assert - all search types documented
        assert!(json.contains("text_search"), "Should document text search");
        assert!(
            json.contains("hybrid_search"),
            "Should document hybrid search"
        );
        assert!(json.contains("batch"), "Should document batch search");
    }

    #[test]
    fn test_create_collection_request_default_metric() {
        // Arrange
        let json = r#"{"name": "test", "dimension": 128}"#;

        // Act
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.metric, "cosine");
    }

    #[test]
    fn test_create_collection_request_with_hamming() {
        // Arrange
        let json = r#"{"name": "test", "dimension": 128, "metric": "hamming"}"#;

        // Act
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.metric, "hamming");
    }

    #[test]
    fn test_create_collection_request_with_jaccard() {
        // Arrange
        let json = r#"{"name": "test", "dimension": 128, "metric": "jaccard"}"#;

        // Act
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.metric, "jaccard");
    }

    #[test]
    fn test_create_collection_request_with_storage_mode() {
        // Arrange
        let json = r#"{"name": "test", "dimension": 128, "storage_mode": "sq8"}"#;

        // Act
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.storage_mode, "sq8");
    }

    #[test]
    fn test_search_request_deserialize() {
        // Arrange
        let json = r#"{"vector": [0.1, 0.2, 0.3], "top_k": 5}"#;

        // Act
        let req: SearchRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.vector, vec![0.1, 0.2, 0.3]);
        assert_eq!(req.top_k, 5);
    }

    #[test]
    fn test_batch_search_request_deserialize() {
        // Arrange
        let json = r#"{"searches": [{"vector": [0.1, 0.2], "top_k": 3}]}"#;

        // Act
        let req: BatchSearchRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.searches.len(), 1);
        assert_eq!(req.searches[0].top_k, 3);
    }

    #[test]
    fn test_text_search_request_deserialize() {
        // Arrange
        let json = r#"{"query": "machine learning", "top_k": 10}"#;

        // Act
        let req: TextSearchRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.query, "machine learning");
        assert_eq!(req.top_k, 10);
    }

    #[test]
    fn test_hybrid_search_request_deserialize() {
        // Arrange
        let json = r#"{"vector": [0.1, 0.2], "query": "test", "top_k": 5}"#;

        // Act
        let req: HybridSearchRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.vector, vec![0.1, 0.2]);
        assert_eq!(req.query, "test");
        assert_eq!(req.top_k, 5);
    }

    #[test]
    fn test_upsert_points_request_deserialize() {
        // Arrange
        let json = r#"{"points": [{"id": 1, "vector": [0.1, 0.2]}]}"#;

        // Act
        let req: UpsertPointsRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(req.points.len(), 1);
        assert_eq!(req.points[0].id, 1);
    }

    #[test]
    fn test_collection_response_serialize() {
        // Arrange
        let resp = CollectionResponse {
            name: "test".to_string(),
            dimension: 128,
            metric: "cosine".to_string(),
            storage_mode: "full".to_string(),
            point_count: 100,
        };

        // Act
        let json = serde_json::to_string(&resp).unwrap();

        // Assert
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"dimension\":128"));
        assert!(json.contains("\"metric\":\"cosine\""));
        assert!(json.contains("\"storage_mode\":\"full\""));
        assert!(json.contains("\"point_count\":100"));
    }

    #[test]
    fn test_search_response_serialize() {
        // Arrange
        let resp = SearchResponse {
            results: vec![SearchResultResponse {
                id: 1,
                score: 0.95,
                payload: None,
            }],
        };

        // Act
        let json = serde_json::to_string(&resp).unwrap();

        // Assert
        assert!(json.contains("\"results\""));
        assert!(json.contains("\"id\":1"));
    }

    #[test]
    fn test_error_response_serialize() {
        // Arrange
        let resp = ErrorResponse {
            error: "Test error".to_string(),
        };

        // Act
        let json = serde_json::to_string(&resp).unwrap();

        // Assert
        assert!(json.contains("\"error\":\"Test error\""));
    }
}
