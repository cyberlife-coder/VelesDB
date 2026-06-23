//! Aggregation query dispatch and execution.
//!
//! Handles detection and execution of GROUP BY / aggregate function queries,
//! routing them to `execute_aggregate` on the appropriate collection.

use axum::{http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;
use velesdb_core::velesql::Query;

use crate::handlers::helpers::notify_query_timing;
use crate::types::{
    AggregationResponse, QueryRequest, QueryResponseMeta, VELESQL_CONTRACT_VERSION,
};
use crate::AppState;

use super::velesql_helpers::{parse_and_validate, velesql_collection_not_found, velesql_error};

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
    params: &std::collections::HashMap<String, serde_json::Value>,
    start: std::time::Instant,
) -> axum::response::Response {
    // Prefer typed vector collection for aggregation.
    let result = if let Some(vc) = state.db.get_vector_collection(collection_name) {
        vc.execute_aggregate(parsed, params)
    } else if let Some(any) = state.db.get_any_collection(collection_name) {
        any.execute_aggregate(parsed, params)
    } else {
        state.operational_metrics.inc_errors();
        return velesql_collection_not_found(collection_name);
    };

    let result = match result {
        Ok(r) => r,
        Err(e) => {
            state.operational_metrics.inc_errors();
            return velesql_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "VELESQL_AGGREGATION_ERROR",
                &e.to_string(),
                "Verify GROUP BY/HAVING clauses and aggregate function arguments",
                None,
            );
        }
    };

    let elapsed = start.elapsed();
    let timing_ms = elapsed.as_secs_f64() * 1000.0;
    notify_query_timing(state, collection_name, start);
    state
        .query_duration_histogram
        .observe(elapsed.as_secs_f64());
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

/// Resolve the collection name for an aggregation query.
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_aggregate_collection(
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
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
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

    if parsed.is_match_query() || !parsed.select.is_aggregation_query() {
        state.operational_metrics.inc_errors();
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
        Err(resp) => {
            state.operational_metrics.inc_errors();
            return resp;
        }
    };

    execute_aggregation_query(&state, &collection_name, &parsed, &req.params, start)
}
