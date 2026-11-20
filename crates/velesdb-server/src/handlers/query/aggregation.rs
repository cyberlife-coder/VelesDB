//! Aggregation query dispatch and execution.
//!
//! Handles detection and execution of GROUP BY / aggregate function queries,
//! routing them to `Database::execute_aggregate`, which applies the read gate.

use axum::{http::StatusCode, response::IntoResponse, Json};
use std::collections::HashMap;
use std::sync::Arc;
use velesdb_core::velesql::Query;

use crate::handlers::helpers::record_query_timing;
use crate::types::{
    AggregationResponse, QueryRequest, QueryResponseMeta, VELESQL_CONTRACT_VERSION,
};
use crate::AppState;

use super::velesql_helpers::{
    parse_and_validate, velesql_collection_not_found, velesql_error, velesql_execution_error,
};

fn aggregation_result_count(result: &serde_json::Value) -> usize {
    match result {
        serde_json::Value::Array(rows) => rows.len(),
        serde_json::Value::Object(_) => 1,
        _ => 0,
    }
}

pub(crate) fn execute_aggregation_query(
    state: &Arc<AppState>,
    collection_name: &str,
    parsed: &Query,
    params: &HashMap<String, serde_json::Value>,
    start: std::time::Instant,
) -> axum::response::Response {
    // Through the database facade, never a collection handle: the facade is
    // where the read gate runs (a denial refuses the aggregate, a scope
    // narrows its rows) and where scalar subqueries are resolved.
    let result = match state.db.execute_aggregate(parsed, params) {
        Ok(r) => r,
        Err(velesdb_core::Error::CollectionNotFound(name)) => {
            state.operational_metrics.inc_errors();
            return velesql_collection_not_found(&name);
        }
        Err(e) => {
            state.operational_metrics.inc_errors();
            return velesql_execution_error(
                &e,
                "VELESQL_AGGREGATION_ERROR",
                "Verify GROUP BY/HAVING clauses and aggregate function arguments",
            );
        }
    };

    let elapsed = start.elapsed();
    let timing_ms = elapsed.as_secs_f64() * 1000.0;
    record_query_timing(state, collection_name, elapsed);
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

/// The refuse-or-proceed phase of [`aggregate`], one rule per step: the
/// query parses and it IS an aggregation (row/search/graph belong on
/// `/query`). Its collection is its `FROM`, which the grammar requires of
/// every `SELECT` (pinned by core's `a_select_without_from_does_not_parse`,
/// beside `velesql::collection_target`). Split out so the handler charges
/// the error metric in ONE place instead of once per refusal arm.
#[allow(clippy::result_large_err)]
fn prepare_aggregation(req: &QueryRequest) -> Result<Query, axum::response::Response> {
    let parsed = parse_and_validate(&req.query)?;
    if parsed.is_match_query() || !parsed.select.is_aggregation_query() {
        return Err(velesql_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VELESQL_AGGREGATION_ERROR",
            "Only aggregation queries are accepted on /aggregate",
            "Use /query for row/search/graph queries; use /aggregate for GROUP BY/aggregate workloads.",
            Some(serde_json::json!({ "endpoint": "/aggregate" })),
        ));
    }
    Ok(parsed)
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
        (status = 500, description = "An engine fault during execution (storage, I/O, internal)", body = crate::types::VelesqlErrorResponse),
        (status = 503, description = "The circuit breaker is open, the database is locked, or a guard rail the engine could not carry (the server's time limit, the memory budget) stopped the statement", body = crate::types::VelesqlErrorResponse),
        (status = 400, description = "Query syntax error", body = crate::types::QueryErrorResponse),
        (status = 422, description = "Aggregation validation/execution error, or a `$name` parameter unbound or of the wrong type (`VELESQL_PARAMETER_ERROR`)", body = crate::types::VelesqlErrorResponse),
        (status = 404, description = "Collection not found", body = crate::types::VelesqlErrorResponse)
    )
)]
pub async fn aggregate(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    state.operational_metrics.inc_queries();

    let parsed = match prepare_aggregation(&req) {
        Ok(parsed) => parsed,
        Err(resp) => {
            state.operational_metrics.inc_errors();
            return resp;
        }
    };

    // The aggregation scan is synchronous core code — run it on the
    // blocking pool so the async workers stay responsive.
    let state_clone = Arc::clone(&state);
    crate::handlers::helpers::run_blocking(move || {
        execute_aggregation_query(
            &state_clone,
            &parsed.select.from,
            &parsed,
            &req.params,
            start,
        )
    })
    .await
    .unwrap_or_else(|resp| resp)
}
