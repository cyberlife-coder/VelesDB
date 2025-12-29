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

use velesdb_core::velesql::{self, Condition, VectorExpr};
use velesdb_core::{Database, DistanceMetric, Point};

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
    /// Vector dimension.
    #[schema(example = 768)]
    pub dimension: usize,
    /// Distance metric (cosine, euclidean, dot).
    #[serde(default = "default_metric")]
    #[schema(example = "cosine")]
    pub metric: String,
}

fn default_metric() -> String {
    "cosine".to_string()
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
}

fn default_vector_weight() -> f32 {
    0.5
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
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid metric: {}", req.metric),
                }),
            )
                .into_response()
        }
    };

    match state.db.create_collection(&req.name, req.dimension, metric) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "message": "Collection created",
                "name": req.name
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

    match collection.search(&req.vector, req.top_k) {
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

    let mut all_results = Vec::with_capacity(req.searches.len());

    for search_req in req.searches {
        match collection.search(&search_req.vector, search_req.top_k) {
            Ok(results) => {
                all_results.push(SearchResponse {
                    results: results
                        .into_iter()
                        .map(|r| SearchResultResponse {
                            id: r.point.id,
                            score: r.score,
                            payload: r.point.payload,
                        })
                        .collect(),
                });
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response()
            }
        }
    }

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;

    Json(BatchSearchResponse {
        results: all_results,
        timing_ms,
    })
    .into_response()
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

    let results = collection.text_search(&req.query, req.top_k);

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

    match collection.hybrid_search(&req.vector, &req.query, req.top_k, Some(req.vector_weight)) {
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
#[allow(clippy::unused_async, clippy::too_many_lines)] // Axum handler requires async, complex query parsing
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

    // Extract vector search and MATCH from WHERE clause
    let vector_search = extract_vector_search(&select.where_clause);
    let match_condition_opt = extract_match_condition(&select.where_clause);

    // Determine limit
    let limit = select.limit.unwrap_or(10) as usize;

    // Execute appropriate search based on conditions
    let results = match (vector_search, match_condition_opt) {
        (Some(vs), Some(match_cond)) => {
            let query_vector = match resolve_vector(&vs.vector, &req.params) {
                Ok(v) => v,
                Err(e) => return bad_request_response(&e),
            };
            match collection.hybrid_search(&query_vector, &match_cond.query, limit, Some(0.5)) {
                Ok(r) => r,
                Err(e) => return bad_request_response(&e.to_string()),
            }
        }
        (Some(vs), None) => {
            let query_vector = match resolve_vector(&vs.vector, &req.params) {
                Ok(v) => v,
                Err(e) => return bad_request_response(&e),
            };
            match collection.search(&query_vector, limit) {
                Ok(r) => r,
                Err(e) => return bad_request_response(&e.to_string()),
            }
        }
        (None, Some(match_cond)) => collection.text_search(&match_cond.query, limit),
        (None, None) => {
            return bad_request_response(
                "VelesQL queries require a search condition (NEAR or MATCH clause)",
            );
        }
    };

    // Apply additional filters if present (post-filtering for non-search conditions)
    let filtered_results: Vec<_> = if select.where_clause.is_some() {
        results
            .into_iter()
            .filter(|r| match_condition(&select.where_clause, &r.point.payload))
            .collect()
    } else {
        results
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    let rows_returned = filtered_results.len();

    Json(QueryResponse {
        results: filtered_results
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

/// Resolve a vector from VectorExpr using query parameters.
fn resolve_vector(
    vector_expr: &VectorExpr,
    params: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<Vec<f32>, String> {
    match vector_expr {
        VectorExpr::Literal(v) => Ok(v.clone()),
        VectorExpr::Parameter(param_name) => match params.get(param_name) {
            Some(serde_json::Value::Array(arr)) => Ok(arr
                .iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect()),
            _ => Err(format!("Missing or invalid parameter: ${param_name}")),
        },
    }
}

/// Extract vector search from a condition tree.
fn extract_vector_search(condition: &Option<Condition>) -> Option<&velesql::VectorSearch> {
    match condition {
        None => None,
        Some(cond) => extract_vector_search_inner(cond),
    }
}

fn extract_vector_search_inner(condition: &Condition) -> Option<&velesql::VectorSearch> {
    match condition {
        Condition::VectorSearch(vs) => Some(vs),
        Condition::And(left, right) => {
            extract_vector_search_inner(left).or_else(|| extract_vector_search_inner(right))
        }
        Condition::Or(left, right) => {
            extract_vector_search_inner(left).or_else(|| extract_vector_search_inner(right))
        }
        Condition::Group(inner) => extract_vector_search_inner(inner),
        _ => None,
    }
}

/// Extract MATCH condition from a condition tree.
fn extract_match_condition(condition: &Option<Condition>) -> Option<&velesql::MatchCondition> {
    match condition {
        None => None,
        Some(cond) => extract_match_inner(cond),
    }
}

fn extract_match_inner(condition: &Condition) -> Option<&velesql::MatchCondition> {
    match condition {
        Condition::Match(m) => Some(m),
        Condition::And(left, right) => {
            extract_match_inner(left).or_else(|| extract_match_inner(right))
        }
        Condition::Or(left, right) => {
            extract_match_inner(left).or_else(|| extract_match_inner(right))
        }
        Condition::Group(inner) => extract_match_inner(inner),
        _ => None,
    }
}

/// Match a condition against a payload (simplified implementation).
fn match_condition(condition: &Option<Condition>, payload: &Option<serde_json::Value>) -> bool {
    match (condition, payload) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(cond), Some(payload_val)) => match_condition_inner(cond, payload_val),
    }
}

fn match_condition_inner(condition: &Condition, payload: &serde_json::Value) -> bool {
    match condition {
        Condition::Comparison(comp) => {
            let field_value = get_nested_value(payload, &comp.column);
            match field_value {
                Some(val) => compare_values(&comp.operator, val, &comp.value),
                None => false,
            }
        }
        Condition::And(left, right) => {
            match_condition_inner(left, payload) && match_condition_inner(right, payload)
        }
        Condition::Or(left, right) => {
            match_condition_inner(left, payload) || match_condition_inner(right, payload)
        }
        Condition::In(in_cond) => {
            let field_value = get_nested_value(payload, &in_cond.column);
            match field_value {
                Some(val) => in_cond.values.iter().any(|v| values_equal(val, v)),
                None => false,
            }
        }
        Condition::IsNull(is_null) => {
            let field_value = get_nested_value(payload, &is_null.column);
            if is_null.is_null {
                field_value.is_none() || field_value == Some(&serde_json::Value::Null)
            } else {
                field_value.is_some() && field_value != Some(&serde_json::Value::Null)
            }
        }
        Condition::Group(inner) => match_condition_inner(inner, payload),
        Condition::VectorSearch(_) => true, // Vector search is handled separately
        _ => true,                          // Other conditions pass through for now
    }
}

fn get_nested_value<'a>(
    payload: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = payload;
    for part in parts {
        match current.get(part) {
            Some(v) => current = v,
            None => return None,
        }
    }
    Some(current)
}

fn compare_values(
    operator: &velesql::CompareOp,
    field: &serde_json::Value,
    value: &velesql::Value,
) -> bool {
    use velesql::CompareOp::*;

    match (field, value) {
        (serde_json::Value::String(f), velesql::Value::String(v)) => match operator {
            Eq => f == v,
            NotEq => f != v,
            Gt => f > v,
            Gte => f >= v,
            Lt => f < v,
            Lte => f <= v,
        },
        (serde_json::Value::Number(f), velesql::Value::Integer(v)) => {
            let f_val = f.as_i64().unwrap_or(0);
            match operator {
                Eq => f_val == *v,
                NotEq => f_val != *v,
                Gt => f_val > *v,
                Gte => f_val >= *v,
                Lt => f_val < *v,
                Lte => f_val <= *v,
            }
        }
        (serde_json::Value::Number(f), velesql::Value::Float(v)) => {
            let f_val = f.as_f64().unwrap_or(0.0);
            match operator {
                Eq => (f_val - v).abs() < f64::EPSILON,
                NotEq => (f_val - v).abs() >= f64::EPSILON,
                Gt => f_val > *v,
                Gte => f_val >= *v,
                Lt => f_val < *v,
                Lte => f_val <= *v,
            }
        }
        (serde_json::Value::Bool(f), velesql::Value::Boolean(v)) => match operator {
            Eq => f == v,
            NotEq => f != v,
            _ => false,
        },
        _ => false,
    }
}

fn values_equal(field: &serde_json::Value, value: &velesql::Value) -> bool {
    match (field, value) {
        (serde_json::Value::String(f), velesql::Value::String(v)) => f == v,
        (serde_json::Value::Number(f), velesql::Value::Integer(v)) => f.as_i64() == Some(*v),
        (serde_json::Value::Number(f), velesql::Value::Float(v)) => f
            .as_f64()
            .map(|fv| (fv - v).abs() < f64::EPSILON)
            .unwrap_or(false),
        (serde_json::Value::Bool(f), velesql::Value::Boolean(v)) => f == v,
        _ => false,
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
}
