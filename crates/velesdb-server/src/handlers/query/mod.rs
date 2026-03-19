//! VelesQL query execution handlers.

pub mod explain;
pub(crate) mod velesql_helpers;

pub use explain::{__path_explain, explain};

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;
use velesdb_core::collection::search::query::projection;
#[cfg(test)]
use velesdb_core::velesql;
use velesdb_core::velesql::{Query, SelectColumns};

use crate::handlers::helpers::notify_query_timing;
use crate::types::{
    AggregationResponse, QueryRequest, QueryResponse, QueryResponseMeta, QueryType,
    VELESQL_CONTRACT_VERSION,
};
use crate::AppState;

use explain::condition_has_vector_search;
use velesql_helpers::{parse_and_validate, velesql_collection_not_found, velesql_error};

fn is_aggregation_query(select: &velesdb_core::velesql::SelectStatement) -> bool {
    let has_aggs = match &select.columns {
        SelectColumns::Aggregations(_) => true,
        SelectColumns::Mixed { aggregations, .. } => !aggregations.is_empty(),
        _ => false,
    };
    has_aggs || select.group_by.is_some()
}

fn aggregation_result_count(result: &serde_json::Value) -> usize {
    match result {
        serde_json::Value::Array(rows) => rows.len(),
        serde_json::Value::Object(_) => 1,
        _ => 0,
    }
}

#[allow(deprecated)]
fn execute_aggregation_query(
    state: &Arc<AppState>,
    collection_name: &str,
    parsed: &Query,
    params: &std::collections::HashMap<String, serde_json::Value>,
    start: std::time::Instant,
) -> axum::response::Response {
    let collection = match state.db.get_collection(collection_name) {
        Some(c) => c,
        None => return velesql_collection_not_found(collection_name),
    };

    let result = match collection.execute_aggregate(parsed, params) {
        Ok(r) => r,
        Err(e) => {
            return velesql_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "VELESQL_AGGREGATION_ERROR",
                &e.to_string(),
                "Verify GROUP BY/HAVING clauses and aggregate function arguments",
                None,
            )
        }
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    notify_query_timing(state, collection_name, start);
    let count = aggregation_result_count(&result);

    Json(AggregationResponse {
        result,
        timing_ms,
        meta: QueryResponseMeta {
            velesql_contract_version: VELESQL_CONTRACT_VERSION.to_string(),
            count,
        },
    })
    .into_response()
}

/// Execute a VelesQL query.
///
/// BUG-1 FIX: Automatically detects aggregation queries (GROUP BY, COUNT, SUM, etc.)
/// and routes them to execute_aggregate for proper handling.
#[utoipa::path(
    post,
    path = "/query",
    tag = "query",
    request_body = QueryRequest,
    responses(
        (status = 200, description = "Query results", body = QueryResponse),
        (status = 400, description = "Query syntax error", body = crate::types::QueryErrorResponse),
        (status = 422, description = "Query validation/execution error", body = crate::types::VelesqlErrorResponse),
        (status = 404, description = "Collection not found", body = crate::types::VelesqlErrorResponse)
    )
)]
#[allow(clippy::unused_async, deprecated)]
pub async fn query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let parsed = match parse_and_validate(&req.query) {
        Ok(q) => q,
        Err(resp) => return resp,
    };

    let collection_name = match resolve_collection_name(&parsed, &req) {
        Ok(name) => name,
        Err(resp) => return resp,
    };

    // BUG-1 FIX: Detect aggregation queries and route to execute_aggregate
    if is_aggregation_query(&parsed.select) {
        return execute_aggregation_query(&state, &collection_name, &parsed, &req.params, start);
    }

    let results = match execute_standard_query(&state, &parsed, &collection_name, &req) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    build_query_response(
        &state,
        &collection_name,
        start,
        results,
        &parsed.select.columns,
    )
}

/// Determine the target collection from the parsed query and request body.
#[allow(clippy::result_large_err)]
fn resolve_collection_name(
    parsed: &Query,
    req: &QueryRequest,
) -> Result<String, axum::response::Response> {
    if parsed.is_match_query() {
        req.collection
            .as_ref()
            .filter(|name| !name.is_empty())
            .cloned()
            .ok_or_else(|| {
                velesql_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "VELESQL_MISSING_COLLECTION",
                    "MATCH query via /query requires `collection` in request body",
                    "Add `collection` to the /query JSON body or use /collections/{name}/match",
                    Some(serde_json::json!({
                        "field": "collection",
                        "endpoint": "/query",
                        "query_type": "MATCH"
                    })),
                )
            })
    } else {
        Ok(parsed.select.from.clone())
    }
}

/// Execute a standard (non-aggregation) query, dispatching MATCH vs SELECT.
#[allow(deprecated, clippy::result_large_err)]
fn execute_standard_query(
    state: &Arc<AppState>,
    parsed: &Query,
    collection_name: &str,
    req: &QueryRequest,
) -> Result<Vec<velesdb_core::SearchResult>, axum::response::Response> {
    let execute_result = if parsed.is_match_query() {
        match state.db.get_collection(collection_name) {
            Some(c) => c.execute_query(parsed, &req.params),
            None => Err(velesdb_core::Error::CollectionNotFound(
                collection_name.to_string(),
            )),
        }
    } else {
        state.db.execute_query(parsed, &req.params)
    };

    execute_result.map_err(|e| match e {
        velesdb_core::Error::CollectionNotFound(name) => velesql_collection_not_found(&name),
        other => velesql_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VELESQL_EXECUTION_ERROR",
            &other.to_string(),
            "Validate query semantics and parameter types against the target collection",
            None,
        ),
    })
}

/// Build the final query response with timing metrics and SQL projection.
fn build_query_response(
    state: &Arc<AppState>,
    collection_name: &str,
    start: std::time::Instant,
    results: Vec<velesdb_core::SearchResult>,
    select_columns: &SelectColumns,
) -> axum::response::Response {
    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    #[allow(clippy::cast_possible_truncation)]
    // Reason: timing_ms is always < u64::MAX (query durations < 585 millennia)
    let took_ms = timing_ms.round() as u64;
    notify_query_timing(state, collection_name, start);
    let projected = projection::project_results(&results, select_columns);
    let rows_returned = projected.len();

    Json(QueryResponse {
        results: projected,
        timing_ms,
        took_ms,
        rows_returned,
        meta: QueryResponseMeta {
            velesql_contract_version: VELESQL_CONTRACT_VERSION.to_string(),
            count: rows_returned,
        },
    })
    .into_response()
}

/// Execute an aggregation-only VelesQL query.
///
/// This endpoint is explicit and stable for GROUP BY / HAVING / aggregate workloads.
#[utoipa::path(
    post,
    path = "/aggregate",
    tag = "query",
    request_body = QueryRequest,
    responses(
        (status = 200, description = "Aggregation results", body = AggregationResponse),
        (status = 400, description = "Query syntax error", body = crate::types::QueryErrorResponse),
        (status = 422, description = "Aggregation validation/execution error", body = crate::types::VelesqlErrorResponse),
        (status = 404, description = "Collection not found", body = crate::types::VelesqlErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn aggregate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let parsed = match parse_and_validate(&req.query) {
        Ok(q) => q,
        Err(resp) => return resp,
    };

    if parsed.is_match_query() || !is_aggregation_query(&parsed.select) {
        return velesql_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VELESQL_AGGREGATION_ERROR",
            "Only aggregation queries are accepted on /aggregate",
            "Use /query for row/search/graph queries; use /aggregate for GROUP BY/aggregate workloads.",
            Some(serde_json::json!({ "endpoint": "/aggregate" })),
        );
    }

    let collection_name = resolve_aggregate_collection(&parsed, &req);
    let collection_name = match collection_name {
        Ok(name) => name,
        Err(resp) => return resp,
    };

    execute_aggregation_query(&state, &collection_name, &parsed, &req.params, start)
}

/// Resolve the collection name for an aggregation query.
#[allow(clippy::result_large_err)]
fn resolve_aggregate_collection(
    parsed: &Query,
    req: &QueryRequest,
) -> Result<String, axum::response::Response> {
    if !parsed.select.from.is_empty() {
        return Ok(parsed.select.from.clone());
    }
    req.collection
        .as_ref()
        .filter(|name| !name.is_empty())
        .cloned()
        .ok_or_else(|| {
            velesql_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "VELESQL_MISSING_COLLECTION",
                "Aggregation query requires a FROM collection or request-body `collection`",
                "Add FROM <collection> to query or set `collection` in request JSON",
                Some(serde_json::json!({
                    "field": "collection",
                    "endpoint": "/aggregate"
                })),
            )
        })
}

/// Detect query type from parsed AST (EPIC-052 US-006).
///
/// Priority order:
/// 1. MATCH clause -> Graph
/// 2. GROUP BY or aggregates -> Aggregation
/// 3. Vector search -> Search
/// 4. Default -> Rows
#[allow(dead_code)] // Used in tests, will be used in unified handler
pub fn detect_query_type(query: &Query) -> QueryType {
    if query.is_match_query() {
        return QueryType::Graph;
    }

    if is_aggregation_query(&query.select) {
        return QueryType::Aggregation;
    }

    let has_vector = query
        .select
        .where_clause
        .as_ref()
        .map(condition_has_vector_search)
        .unwrap_or(false);

    if has_vector {
        return QueryType::Search;
    }

    QueryType::Rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_query_type_search() {
        let parsed = velesql::Parser::parse(
            "SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10",
        )
        .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Search);
    }

    #[test]
    fn test_detect_query_type_aggregation() {
        let parsed =
            velesql::Parser::parse("SELECT category, COUNT(*) FROM products GROUP BY category")
                .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Aggregation);
    }

    #[test]
    fn test_detect_query_type_rows() {
        let parsed =
            velesql::Parser::parse("SELECT name, price FROM products WHERE price > 100").unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Rows);
    }

    #[test]
    fn test_detect_query_type_graph() {
        let parsed =
            velesql::Parser::parse("MATCH (n:Person)-[:KNOWS]->(m) RETURN n.name, m.name LIMIT 10")
                .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Graph);
    }

    #[test]
    fn test_detect_query_type_hybrid_vector_aggregation() {
        // When both vector search and aggregation, aggregation takes priority
        let parsed = velesql::Parser::parse(
            "SELECT category, COUNT(*) FROM docs WHERE similarity(embedding, $v) > 0.7 GROUP BY category",
        )
        .unwrap();
        assert_eq!(detect_query_type(&parsed), QueryType::Aggregation);
    }
}
