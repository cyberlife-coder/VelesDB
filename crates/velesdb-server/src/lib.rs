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
//! `VelesDB` Server - REST API library for the `VelesDB` vector database.
//!
//! This module provides the HTTP handlers and types for the `VelesDB` REST API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use velesdb_core::velesql::{self, Condition, VectorExpr};
use velesdb_core::{Database, DistanceMetric, Point};

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

/// Request for `VelesQL` query execution.
#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    /// The `VelesQL` query string.
    pub query: String,
    /// Named parameters for the query.
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
}

/// Response from VelesQL query execution.
#[derive(Debug, Serialize)]
pub struct QueryResponse {
    /// Query results.
    pub results: Vec<SearchResultResponse>,
    /// Query execution time in milliseconds.
    pub timing_ms: f64,
    /// Number of rows returned.
    pub rows_returned: usize,
}

/// VelesQL query error response.
#[derive(Debug, Serialize)]
pub struct QueryErrorResponse {
    /// Error details.
    pub error: QueryErrorDetail,
}

/// VelesQL query error detail.
#[derive(Debug, Serialize)]
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

/// Execute a VelesQL query.
///
/// POST /query
/// ```json
/// {
///   "query": "SELECT * FROM documents WHERE vector NEAR $v LIMIT 10",
///   "params": { "v": [0.1, 0.2, ...] }
/// }
/// ```
#[allow(clippy::unused_async)] // Axum handler requires async
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

    // Extract vector search from WHERE clause
    let vector_search = extract_vector_search(&select.where_clause);

    // Extract vector from params if needed
    let query_vector = match vector_search {
        Some(vs) => match &vs.vector {
            VectorExpr::Literal(v) => v.clone(),
            VectorExpr::Parameter(param_name) => match req.params.get(param_name) {
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                    .collect(),
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Missing or invalid parameter: ${}", param_name),
                        }),
                    )
                        .into_response()
                }
            },
        },
        None => {
            // No vector search, return error for now
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "VelesQL queries require a vector search (NEAR clause)".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Determine limit
    let limit = select.limit.unwrap_or(10) as usize;

    // Execute search
    let results = match collection.search(&query_vector, limit) {
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

    // Apply filters if present (post-filtering)
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
