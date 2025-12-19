//! VelesDB Server - REST API library for the VelesDB vector database.
//!
//! This module provides the HTTP handlers and types for the VelesDB REST API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use velesdb_core::{Database, DistanceMetric, Point};

// ============================================================================
// Application State
// ============================================================================

/// Application state shared across handlers.
pub struct AppState {
    /// The VelesDB database instance.
    pub db: Database,
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request to create a new collection.
#[derive(Debug, Deserialize)]
pub struct CreateCollectionRequest {
    /// Collection name.
    pub name: String,
    /// Vector dimension.
    pub dimension: usize,
    /// Distance metric (cosine, euclidean, dot).
    #[serde(default = "default_metric")]
    pub metric: String,
}

fn default_metric() -> String {
    "cosine".to_string()
}

/// Response with collection information.
#[derive(Debug, Serialize)]
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
#[derive(Debug, Deserialize)]
pub struct UpsertPointsRequest {
    /// Points to upsert.
    pub points: Vec<PointRequest>,
}

/// A point in an upsert request.
#[derive(Debug, Deserialize)]
pub struct PointRequest {
    /// Point ID.
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Optional payload.
    pub payload: Option<serde_json::Value>,
}

/// Request for vector search.
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    /// Query vector.
    pub vector: Vec<f32>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

/// Request for batch vector search.
#[derive(Debug, Deserialize)]
pub struct BatchSearchRequest {
    /// List of search requests.
    pub searches: Vec<SearchRequest>,
}

fn default_top_k() -> usize {
    10
}

/// Response from vector search.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    /// Search results.
    pub results: Vec<SearchResultResponse>,
}

/// Response from batch search.
#[derive(Debug, Serialize)]
pub struct BatchSearchResponse {
    /// Results for each search query.
    pub results: Vec<SearchResponse>,
    /// Total time in milliseconds.
    pub timing_ms: f64,
}

/// A single search result.
#[derive(Debug, Serialize)]
pub struct SearchResultResponse {
    /// Point ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Point payload.
    pub payload: Option<serde_json::Value>,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error message.
    pub error: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// Health check endpoint.
pub async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// List all collections.
pub async fn list_collections(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let collections = state.db.list_collections();
    Json(serde_json::json!({ "collections": collections }))
}

/// Create a new collection.
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

    let count = points.len();

    match collection.upsert(points) {
        Ok(()) => Json(serde_json::json!({
            "message": "Points upserted",
            "count": count
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

/// Get a point by ID.
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
