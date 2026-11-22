//! VelesQL query execution handlers.

pub mod aggregation;
pub mod explain;
pub(crate) mod velesql_helpers;

pub use aggregation::__path_aggregate;
pub use aggregation::aggregate;
pub use explain::{__path_explain, explain};

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;
use velesdb_core::collection::search::query::projection;
use velesdb_core::velesql::{DmlStatement, Query};

use crate::types::{QueryRequest, QueryResponse, QueryResponseMeta, VELESQL_CONTRACT_VERSION};
use crate::AppState;

use crate::handlers::helpers::run_blocking;
use aggregation::execute_aggregation_query;
use velesql_helpers::{
    parse_and_validate, velesql_collection_not_found, velesql_error, velesql_execution_error,
};

/// Returns `true` when the query should bypass collection resolution and go
/// directly through `Database::execute_query` — DDL, introspection, admin,
/// TRAIN, or graph/edge/delete DML that resolves its own collection from the AST.
fn requires_mutation_dispatch(parsed: &Query) -> bool {
    parsed.is_ddl_query()
        || parsed.is_introspection_query()
        || parsed.is_admin_query()
        || parsed.is_train()
        || is_ast_routed_dml(parsed)
}

/// Returns `true` for DML statements that resolve their collection name from
/// the AST rather than from the request body's `FROM` clause:
/// `INSERT EDGE`, `DELETE`, `DELETE EDGE`, `SELECT EDGES`, `INSERT NODE`.
///
/// `INSERT INTO`, `UPSERT`, and `UPDATE` return result rows and must flow
/// through the standard query path (they use `stmt.table` which maps to
/// the SELECT `FROM`).
fn is_ast_routed_dml(parsed: &Query) -> bool {
    matches!(
        parsed.dml,
        Some(
            DmlStatement::InsertEdge(_)
                | DmlStatement::Delete(_)
                | DmlStatement::DeleteEdge(_)
                | DmlStatement::SelectEdges(_)
                | DmlStatement::InsertNode(_)
        )
    )
}

/// Execute a VelesQL query.
///
/// BUG-1 FIX: Automatically detects aggregation queries (GROUP BY, COUNT, SUM, etc.)
/// and routes them to execute_aggregate for proper handling.
///
/// DDL statements (CREATE/DROP COLLECTION) are intercepted before collection
/// resolution and dispatched directly through `Database::execute_query`.
#[utoipa::path(
    post,
    path = "/query",
    tag = "query",
    request_body = QueryRequest,
    responses(
        (status = 200, description = "Query results", body = QueryResponse),
        (status = 500, description = "An engine fault during execution (storage, I/O, internal)", body = crate::types::VelesqlErrorResponse),
        (status = 503, description = "The circuit breaker is open, the database is locked, or a guard rail the engine could not carry (the server's time limit, the memory budget) stopped the statement", body = crate::types::VelesqlErrorResponse),
        (status = 400, description = "Query syntax error", body = crate::types::QueryErrorResponse),
        (status = 422, description = "Query validation/execution error, or a `$name` parameter unbound or of the wrong type (`VELESQL_PARAMETER_ERROR`)", body = crate::types::VelesqlErrorResponse),
        (status = 404, description = "Collection not found", body = crate::types::VelesqlErrorResponse)
    )
)]
pub async fn query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    state.operational_metrics.inc_queries();

    let parsed = match parse_and_validate(&req.query) {
        Ok(q) => q,
        Err(resp) => {
            state.operational_metrics.inc_errors();
            return resp;
        }
    };

    // Query execution calls synchronous core code (locks, and fsync on the
    // DML/DDL paths) — run it on the blocking pool so the async workers stay
    // responsive, mirroring the points/search handler discipline.
    let state_clone = Arc::clone(&state);
    run_blocking(move || dispatch_parsed_query(&state_clone, &parsed, &req, start))
        .await
        .unwrap_or_else(|resp| resp)
}

/// Synchronous post-parse dispatch for [`query`]: routes the parsed query to
/// the mutation, aggregation, or standard execution path and builds the
/// response. Runs on the blocking pool.
fn dispatch_parsed_query(
    state: &Arc<AppState>,
    parsed: &Query,
    req: &QueryRequest,
    start: std::time::Instant,
) -> axum::response::Response {
    // DDL/Introspection/Admin/graph-mutation bypass: these extract collection from
    // the SQL AST, not from the request body.  INSERT INTO, UPSERT, and UPDATE flow
    // through the standard path because they return meaningful result rows.
    if requires_mutation_dispatch(parsed) {
        return execute_mutation_query(state, parsed, &req.params, start);
    }

    // The body's `collection` fills `_collection` for a top-level MATCH when
    // the caller set none: core's one rule, shared with the Tauri plugin.
    let params = velesdb_core::velesql::with_collection_param(
        parsed,
        &req.params,
        req.collection.as_deref(),
    );
    let collection_name = match resolve_collection_name(parsed, &params) {
        Ok(name) => name,
        Err(resp) => {
            state.operational_metrics.inc_errors();
            return resp;
        }
    };

    // BUG-1 FIX: Detect aggregation queries and route to execute_aggregate
    if parsed.select.is_aggregation_query() {
        return execute_aggregation_query(state, &collection_name, parsed, &params, start);
    }

    let results = match execute_standard_query(state, parsed, &params) {
        Ok(r) => r,
        Err(resp) => {
            state.operational_metrics.inc_errors();
            return resp;
        }
    };

    build_query_response(state, start, results, parsed)
}

/// Execute a DDL, graph/delete DML, introspection, admin, or TRAIN query.
///
/// DDL (CREATE/DROP/ALTER/ANALYZE/TRUNCATE), graph/delete DML mutations
/// (INSERT EDGE, DELETE, DELETE EDGE, SELECT EDGES, INSERT NODE),
/// introspection (SHOW/DESCRIBE/EXPLAIN), admin (FLUSH), and TRAIN
/// statements extract collection names from the SQL AST — no FROM clause
/// needed.
///
/// Results from `Database::execute_query` are propagated into the response
/// so that introspection (SHOW, DESCRIBE, EXPLAIN), ANALYZE, and SELECT
/// EDGES return their data to the caller.
fn execute_mutation_query(
    state: &Arc<AppState>,
    parsed: &Query,
    params: &std::collections::HashMap<String, serde_json::Value>,
    start: std::time::Instant,
) -> axum::response::Response {
    match state.db.execute_query(parsed, params) {
        Ok(results) => build_query_response(state, start, results, parsed),
        Err(e) => {
            state.operational_metrics.inc_errors();
            velesql_execution_error(
                &e,
                "VELESQL_MUTATION_ERROR",
                "Check collection name, statement syntax, and target existence",
            )
        }
    }
}

/// The collection `parsed` reads, by core's rule
/// ([`velesdb_core::velesql::collection_target`]): its `FROM`, else, for a
/// top-level `MATCH`, the `_collection` of `params` (filled from the body's
/// `collection` when the caller set none). A `_collection` that is no
/// collection name is core's parameter error (`VELESQL_PARAMETER_ERROR`); a
/// top-level `MATCH` naming none is refused; a write on this path (`INSERT
/// INTO`, `UPSERT`, `UPDATE`) names its table in the statement and resolves
/// to no name here.
#[allow(clippy::result_large_err)]
fn resolve_collection_name(
    parsed: &Query,
    params: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, axum::response::Response> {
    match velesdb_core::velesql::collection_target(parsed, params) {
        Ok(Some(name)) => return Ok(name.to_string()),
        Ok(None) if !parsed.is_match_query() => return Ok(String::new()),
        Ok(None) => {}
        Err(e) => return Err(velesql_execution_error(&e, "VELESQL_PARAMETER_ERROR", "")),
    }
    Err(velesql_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "VELESQL_MISSING_COLLECTION",
        "MATCH query via /query requires `collection` in request body or `_collection` in params",
        "Add `collection` to the /query JSON body or use /collections/{name}/match",
        Some(serde_json::json!({
            "field": "collection",
            "endpoint": "/query",
            "query_type": "MATCH"
        })),
    ))
}

/// Execute a standard (non-aggregation) query, a MATCH or a SELECT, with the
/// parameters its `collection` filled (`velesql::with_collection_param`).
#[allow(clippy::result_large_err)]
fn execute_standard_query(
    state: &Arc<AppState>,
    parsed: &Query,
    params: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<Vec<velesdb_core::SearchResult>, axum::response::Response> {
    state.db.execute_query(parsed, params).map_err(|e| match e {
        velesdb_core::Error::CollectionNotFound(name) => velesql_collection_not_found(&name),
        other => velesql_execution_error(
            &other,
            "VELESQL_EXECUTION_ERROR",
            "Validate query semantics and parameter types against the target collection",
        ),
    })
}

/// Build the final query response with timing metrics and SQL projection.
///
/// Both callers ([`execute_standard_query`] and [`execute_mutation_query`])
/// dispatch through [`Database::execute_query`](velesdb_core::Database::execute_query),
/// which already fires the observer's `on_query` telemetry exactly once
/// internally, as every gated read does. Only the Prometheus histogram,
/// which is unrelated to the observer, is recorded here.
fn build_query_response(
    state: &Arc<AppState>,
    start: std::time::Instant,
    results: Vec<velesdb_core::SearchResult>,
    query: &Query,
) -> axum::response::Response {
    let elapsed = start.elapsed();
    let timing_ms = elapsed.as_secs_f64() * 1000.0;
    #[allow(clippy::cast_possible_truncation)]
    // Reason: timing_ms is always < u64::MAX (query durations < 585 millennia)
    let took_ms = timing_ms.round() as u64;
    state
        .query_duration_histogram
        .observe(elapsed.as_secs_f64());
    let projected = projection::project_query_results(&results, query);
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
