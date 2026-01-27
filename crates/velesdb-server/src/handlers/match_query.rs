//! MATCH query handler for REST API (EPIC-045 US-007).
//!
//! Provides endpoint for executing graph pattern matching queries.

#![allow(dead_code)] // Handler to be wired in routes in future PR

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;

/// Request body for MATCH query execution.
#[derive(Debug, Deserialize)]
pub struct MatchQueryRequest {
    /// VelesQL MATCH query string.
    pub query: String,
    /// Query parameters (e.g., vectors, values).
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

/// Single result from MATCH query.
#[derive(Debug, Serialize)]
pub struct MatchQueryResultItem {
    /// Variable bindings from pattern matching.
    pub bindings: HashMap<String, u64>,
    /// Similarity score (if similarity() was used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// Traversal depth.
    pub depth: u32,
}

/// Response for MATCH query execution.
#[derive(Debug, Serialize)]
pub struct MatchQueryResponse {
    /// Query results.
    pub results: Vec<MatchQueryResultItem>,
    /// Execution time in milliseconds.
    pub took_ms: u64,
    /// Number of results.
    pub count: usize,
}

/// Error response for MATCH query.
#[derive(Debug, Serialize)]
pub struct MatchQueryError {
    /// Error message.
    pub error: String,
    /// Error code.
    pub code: String,
}

/// Execute a MATCH query on a collection.
///
/// # Endpoint
///
/// `POST /collections/{name}/match`
///
/// # Example Request
///
/// ```json
/// {
///   "query": "MATCH (a:Person)-[:KNOWS]->(b) WHERE similarity(a.vec, $v) > 0.8 RETURN a.name",
///   "params": {
///     "v": [0.1, 0.2, 0.3]
///   }
/// }
/// ```
pub async fn match_query(
    Path(collection_name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(request): Json<MatchQueryRequest>,
) -> Result<Json<MatchQueryResponse>, (StatusCode, Json<MatchQueryError>)> {
    let start = std::time::Instant::now();

    // Get collection
    let collection = state.db.get_collection(&collection_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(MatchQueryError {
                error: format!("Collection '{}' not found", collection_name),
                code: "COLLECTION_NOT_FOUND".to_string(),
            }),
        )
    })?;

    // Parse MATCH query
    let query = velesdb_core::velesql::Parser::parse(&request.query).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(MatchQueryError {
                error: format!("Parse error: {}", e),
                code: "PARSE_ERROR".to_string(),
            }),
        )
    })?;

    // Verify it's a MATCH query
    let match_clause = query.match_clause.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(MatchQueryError {
                error: "Query is not a MATCH query".to_string(),
                code: "NOT_MATCH_QUERY".to_string(),
            }),
        )
    })?;

    // Execute MATCH query
    let results = collection
        .execute_match(&match_clause, &request.params)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(MatchQueryError {
                    error: format!("Execution error: {}", e),
                    code: "EXECUTION_ERROR".to_string(),
                }),
            )
        })?;

    // Convert results
    let result_items: Vec<MatchQueryResultItem> = results
        .into_iter()
        .map(|r| MatchQueryResultItem {
            bindings: r.bindings,
            score: r.score,
            depth: r.depth,
        })
        .collect();

    let count = result_items.len();
    let took_ms = start.elapsed().as_millis() as u64;

    Ok(Json(MatchQueryResponse {
        results: result_items,
        took_ms,
        count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_query_request_deserialize() {
        let json = r#"{
            "query": "MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name",
            "params": {}
        }"#;

        let request: MatchQueryRequest = serde_json::from_str(json).unwrap();
        assert!(request.query.contains("MATCH"));
        assert!(request.params.is_empty());
    }

    #[test]
    fn test_match_query_response_serialize() {
        let response = MatchQueryResponse {
            results: vec![MatchQueryResultItem {
                bindings: HashMap::from([("a".to_string(), 123)]),
                score: Some(0.95),
                depth: 1,
            }],
            took_ms: 15,
            count: 1,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("bindings"));
        assert!(json.contains("0.95"));
    }
}
