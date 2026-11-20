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
    /// Score threshold for `vector`, in the collection metric's direction: a
    /// row is kept when its score is at least this under a similarity metric
    /// (`cosine`, `dot`, `jaccard`), at most this under a distance metric
    /// (`euclidean`, `hamming`). Any finite number, sent with `vector`
    /// (without one no row is scored, and it is refused); omitted, every
    /// scored row is kept.
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
/// When the query fails with a core error, the JSON body carries its
/// `VELES-XXX` code, and the status follows its variant:
/// - `404 NOT_FOUND` (`VELES-002`) — collection not found
/// - `400 BAD_REQUEST` (`VELES-010`) — parse error, not a MATCH query,
///   a threshold that is not finite or comes without `vector`, or an
///   `ORDER BY` MATCH cannot sort by (`V015`, refused before traversal as
///   on `/query`)
/// - `400 BAD_REQUEST` (`VELES-038`) — a `$name` parameter that is unbound,
///   or bound to a value its position cannot take
/// - otherwise: `404 NOT_FOUND` for a missing point, edge or node;
///   `409 CONFLICT` for a collection or edge that already exists;
///   `400 BAD_REQUEST` for other invalid input and for a depth, cardinality
///   or capacity ceiling the query crossed; `429 TOO_MANY_REQUESTS` for the
///   rate limit; `503 SERVICE_UNAVAILABLE` for a locked database, an open
///   circuit breaker, the memory budget or a timeout under the server's
///   limit; `500 INTERNAL_SERVER_ERROR` for anything else
#[utoipa::path(
    post,
    path = "/collections/{name}/match",
    tag = "graph",
    params(("name" = String, Path, description = "Collection name")),
    request_body = MatchQueryRequest,
    responses(
        (status = 200, description = "Match query results", body = MatchQueryResponse),
        (status = 400, description = "Parse error or invalid query (VELES-010), or a `$name` parameter unbound or of the wrong type (VELES-038)", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
        (status = 429, description = "The client's rate limit", body = ErrorResponse),
        (status = 503, description = "The database is locked, the circuit breaker is open, or a guard rail the engine could not carry (the server's time limit, the memory budget) stopped the query", body = ErrorResponse),
        (status = 422, response = crate::types::MalformedBody)
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

    if !is_match_target(state, collection_name) {
        return Err(Error::CollectionNotFound(collection_name.to_string()));
    }

    let match_clause = parse_match_clause(&request.query)?;
    let scoring = velesdb_core::MatchScoring::new(request.vector.as_deref(), request.threshold)?;

    // Core gates the read (CORE-2) — the target and every `@collection` the
    // pattern names, each a graph-traversal read allowed unscoped — before
    // anything is traversed, then runs the MATCH as `/query` runs it.
    let rows = state
        .db
        .match_query(collection_name, &match_clause, &request.params, scoring)?;
    // Counted once the read is admitted and run, as on the traversal
    // handlers.
    state.operational_metrics.record_graph_query();

    let results: Vec<MatchQueryResultItem> = rows
        .into_iter()
        .map(|r| MatchQueryResultItem {
            bindings: r.bindings,
            score: r.score,
            depth: r.depth,
            projected: r.projected,
        })
        .collect();

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

/// Whether `name` is a collection MATCH can target: a vector or a graph
/// collection (a metadata collection has no graph to traverse).
fn is_match_target(state: &AppState, name: &str) -> bool {
    state.db.get_vector_collection(name).is_some() || state.db.get_graph_collection(name).is_some()
}

#[cfg(test)]
#[path = "match_query_tests.rs"]
mod tests;
