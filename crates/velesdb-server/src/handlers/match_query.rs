//! MATCH query handler for REST API (EPIC-045 US-007).
//!
//! Provides endpoint for executing graph pattern matching queries.

// EPIC-058 US-007: MATCH query handler now wired to /collections/{name}/match

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;
use velesdb_core::api_types::serde_id;
use velesdb_core::Error;

use crate::handlers::helpers::auto_core_error_response;
use crate::types::{ErrorResponse, VELESQL_CONTRACT_VERSION};
use crate::AppState;

/// Request body for MATCH query execution.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MatchQueryRequest {
    /// VelesQL MATCH query string.
    pub query: String,
    /// Query parameters (e.g., vectors, values).
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    /// Query vector for similarity scoring (EPIC-058 US-007).
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    /// Similarity threshold (0.0 to 1.0, default 0.0).
    #[serde(default)]
    pub threshold: Option<f32>,
}

/// Single result from MATCH query.
#[derive(Debug, Serialize, ToSchema)]
pub struct MatchQueryResultItem {
    /// Variable bindings from pattern matching.
    #[serde(serialize_with = "serde_id::serialize_id_map_as_strings")]
    #[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_map_schema))]
    pub bindings: HashMap<String, u64>,
    /// Similarity score (if similarity() was used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// Traversal depth.
    pub depth: u32,
    /// Projected properties from RETURN clause (EPIC-058 US-007).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub projected: HashMap<String, serde_json::Value>,
}

/// Response for MATCH query execution.
#[derive(Debug, Serialize, ToSchema)]
pub struct MatchQueryResponse {
    /// Query results.
    pub results: Vec<MatchQueryResultItem>,
    /// Execution time in milliseconds.
    pub took_ms: u64,
    /// Number of results.
    pub count: usize,
    /// Response metadata.
    pub meta: MatchQueryMeta,
}

/// Metadata section for MATCH query responses.
#[derive(Debug, Serialize, ToSchema)]
pub struct MatchQueryMeta {
    /// VelesQL contract version used by this response.
    pub velesql_contract_version: String,
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
///
/// # Errors
///
/// All failures are mapped through the canonical `auto_core_error_response`,
/// so the JSON body carries the `VELES-XXX` code and the HTTP status is
/// derived from the core error variant:
/// - `404 NOT_FOUND` (`VELES-002`) — collection not found
/// - `400 BAD_REQUEST` (`VELES-010`) — parse error, not a MATCH query,
///   invalid threshold, or an unbound query parameter
/// - other core variants map per [`super::helpers::http_status_for_error`]
#[utoipa::path(
    post,
    path = "/collections/{name}/match",
    tag = "graph",
    params(("name" = String, Path, description = "Collection name")),
    request_body = MatchQueryRequest,
    responses(
        (status = 200, description = "Match query results", body = MatchQueryResponse),
        (status = 400, description = "Parse error or invalid query", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    )
)]
pub async fn match_query(
    Path(collection_name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(request): Json<MatchQueryRequest>,
) -> axum::response::Response {
    // MATCH execution is a synchronous graph traversal (lock-taking core
    // code) — run it on the blocking pool so the async workers stay
    // responsive.
    let state_clone = Arc::clone(&state);
    let outcome = crate::handlers::helpers::run_blocking(move || {
        run_match(&state_clone, &collection_name, &request)
    })
    .await;
    match outcome {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(e)) => auto_core_error_response(&e),
        Err(resp) => resp,
    }
}

/// Resolve, parse, validate, and execute a MATCH request, surfacing every
/// failure as a `velesdb_core::Error` so the handler can route it through
/// `auto_core_error_response` (canonical VELES code + HTTP status).
fn run_match(
    state: &AppState,
    collection_name: &str,
    request: &MatchQueryRequest,
) -> Result<MatchQueryResponse, Error> {
    let start = std::time::Instant::now();

    let collection = resolve_match_collection(state, collection_name)
        .ok_or_else(|| Error::CollectionNotFound(collection_name.to_string()))?;

    let match_clause = parse_match_clause(&request.query)?;
    validate_threshold(request.threshold)?;

    // Gate the read (CORE-2). MATCH is a graph-traversal read; a `?`-propagated
    // Deny refuses it, and a scope narrowing (no filter channel here) fails
    // closed.
    if state
        .db
        .authorize_read(
            collection_name,
            velesdb_core::observer::QueryOperationKind::GraphTraversal,
            None,
            None,
        )?
        .is_some()
    {
        return Err(Error::Config(
            "scope narrowing is not supported for MATCH queries".to_string(),
        ));
    }

    let results = execute_match(&collection, &match_clause, request)?;

    let count = results.len();
    #[allow(clippy::cast_possible_truncation)]
    let took_ms = start.elapsed().as_millis() as u64;

    Ok(MatchQueryResponse {
        results,
        took_ms,
        count,
        meta: MatchQueryMeta {
            velesql_contract_version: VELESQL_CONTRACT_VERSION.to_string(),
        },
    })
}

/// Parse a query string and extract the MATCH clause.
///
/// Both a syntax error and a non-MATCH query are client-side query mistakes,
/// so they map to `Error::Query` (`VELES-010`, 400).
fn parse_match_clause(query_str: &str) -> Result<velesdb_core::velesql::MatchClause, Error> {
    let query = velesdb_core::velesql::Parser::parse(query_str)?;
    query.match_clause.ok_or_else(|| {
        Error::Query(
            "Query is not a MATCH query. Use MATCH (...) RETURN ... \
             or call /query for SELECT statements."
                .to_string(),
        )
    })
}

/// Validate that threshold (if provided) is in [0.0, 1.0].
fn validate_threshold(threshold: Option<f32>) -> Result<(), Error> {
    if let Some(t) = threshold {
        if !(0.0..=1.0).contains(&t) {
            return Err(Error::Query(format!(
                "Invalid threshold: {t}. Must be between 0.0 and 1.0"
            )));
        }
    }
    Ok(())
}

enum MatchCollection {
    Vector(velesdb_core::collection::VectorCollection),
    Graph(velesdb_core::collection::GraphCollection),
}

fn resolve_match_collection(state: &AppState, name: &str) -> Option<MatchCollection> {
    state
        .db
        .get_vector_collection(name)
        .map(MatchCollection::Vector)
        .or_else(|| {
            state
                .db
                .get_graph_collection(name)
                .map(MatchCollection::Graph)
        })
}

fn execute_match(
    collection: &MatchCollection,
    match_clause: &velesdb_core::velesql::MatchClause,
    request: &MatchQueryRequest,
) -> Result<Vec<MatchQueryResultItem>, Error> {
    let raw_results = if let Some(ref vector) = request.vector {
        let threshold = request.threshold.unwrap_or(0.0);
        match collection {
            MatchCollection::Vector(coll) => {
                coll.execute_match_with_similarity(match_clause, vector, threshold, &request.params)
            }
            MatchCollection::Graph(coll) => {
                coll.execute_match_with_similarity(match_clause, vector, threshold, &request.params)
            }
        }
    } else {
        match collection {
            MatchCollection::Vector(coll) => coll.execute_match(match_clause, &request.params),
            MatchCollection::Graph(coll) => coll.execute_match(match_clause, &request.params),
        }
    };

    raw_results.map(|results| {
        results
            .into_iter()
            .map(|r| MatchQueryResultItem {
                bindings: r.bindings,
                score: r.score,
                depth: r.depth,
                projected: r.projected,
            })
            .collect()
    })
}

#[cfg(test)]
#[path = "match_query_tests.rs"]
mod tests;
