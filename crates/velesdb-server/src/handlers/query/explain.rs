//! EXPLAIN query handler and plan building logic.

use axum::{extract::State, response::IntoResponse, Json};
use std::sync::Arc;
use velesdb_core::velesql::QueryPlan;

use crate::types::{
    ActualStatsResponse, ExplainCost, ExplainFeatures, ExplainRequest, ExplainResponse,
    ExplainStep, NodeStatsResponse,
};
use crate::AppState;

use super::velesql_helpers::{
    parse_and_validate, velesql_collection_not_found, velesql_execution_error,
};
use velesdb_core::Error as CoreError;

/// Explain a VelesQL query, optionally executing it with instrumentation.
///
/// When `analyze` is false (default), returns the estimated plan only.
/// When `analyze` is true, executes the query and returns actual statistics.
#[utoipa::path(
    post,
    path = "/query/explain",
    tag = "query",
    request_body = ExplainRequest,
    responses(
        (status = 200, description = "Query plan", body = ExplainResponse),
        (status = 500, description = "An engine fault during execution (storage, I/O, internal)", body = crate::types::VelesqlErrorResponse),
        (status = 503, description = "The circuit breaker is open, the database is locked, or a guard rail the engine could not carry (the server's time limit, the memory budget) stopped the statement", body = crate::types::VelesqlErrorResponse),
        (status = 400, description = "Query syntax error", body = crate::types::QueryErrorResponse),
        (status = 422, description = "Query validation/execution error", body = crate::types::VelesqlErrorResponse),
        (status = 404, description = "Collection not found", body = crate::types::VelesqlErrorResponse)
    )
)]
pub async fn explain(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExplainRequest>,
) -> impl IntoResponse {
    let parsed = match parse_and_validate(&req.query) {
        Ok(q) => q,
        Err(resp) => return resp,
    };

    // Plan building and (with `analyze`) query execution call synchronous
    // core code that can take locks — run them on the blocking pool so the
    // async workers stay responsive.
    let state_clone = Arc::clone(&state);
    crate::handlers::helpers::run_blocking(move || explain_parsed(&state_clone, &req, &parsed))
        .await
        .unwrap_or_else(|resp| resp)
}

/// Synchronous post-parse body of [`explain`]. Runs on the blocking pool.
fn explain_parsed(
    state: &AppState,
    req: &ExplainRequest,
    parsed: &velesdb_core::velesql::Query,
) -> axum::response::Response {
    let select = &parsed.select;

    let collection_exists = state.db.get_any_collection(&select.from).is_some();
    if !collection_exists && !select.from.is_empty() {
        return velesql_collection_not_found(&select.from);
    }

    if req.analyze {
        return explain_with_analyze(state, req, parsed);
    }

    explain_plan_only(state, req, parsed)
}

/// The query-type label shared by the plan-only and ANALYZE paths.
fn query_type(parsed: &velesdb_core::velesql::Query) -> &'static str {
    if parsed.is_match_query() {
        "MATCH"
    } else {
        "SELECT"
    }
}

/// Build an EXPLAIN-only response (no execution).
fn explain_plan_only(
    state: &AppState,
    req: &ExplainRequest,
    parsed: &velesdb_core::velesql::Query,
) -> axum::response::Response {
    let select = &parsed.select;
    let query_type = query_type(parsed);

    // Single-sourced from core: the plan steps come from the canonical
    // `QueryPlan`, not a server-side AST reconstruction, and the features /
    // estimated-cost sections derive from that same plan — nothing is
    // re-derived from the AST server-side (#2678). `explain_query`
    // runs the read gate in-core with the query's own operation kind — a
    // denial or an unnarrowable scope is an error it returns, and it is
    // surfaced as one: a failure folded into a plan payload would answer
    // HTTP 200 for a plan the caller was refused (#2587). The request's
    // params go through so a `MATCH` whose target rides in `_collection` is
    // gated on the collection it actually traverses (#2598).
    let (plan, cache_hit, plan_reuse_count, features, estimated_cost) =
        match state.db.explain_query_with_params(parsed, &req.params) {
            Ok(qp) => (
                core_plan_steps(&qp),
                qp.cache_hit,
                qp.plan_reuse_count,
                ExplainFeatures::from_plan(&qp),
                ExplainCost::from_plan(&qp),
            ),
            Err(e) => {
                return velesql_execution_error(
                    &e,
                    "VELESQL_EXPLAIN_ERROR",
                    "The read gate refused the plan or the query failed plan-time checks",
                );
            }
        };

    Json(ExplainResponse {
        query: req.query.clone(),
        query_type: query_type.to_string(),
        collection: select.from.clone(),
        plan,
        estimated_cost,
        features,
        cache_hit,
        plan_reuse_count,
        estimated_cost_ms: None,
        actual_time_ms: None,
        actual_stats: None,
        node_stats: None,
    })
    .into_response()
}

/// Build an EXPLAIN ANALYZE response (with execution and actual stats).
fn explain_with_analyze(
    state: &AppState,
    req: &ExplainRequest,
    parsed: &velesdb_core::velesql::Query,
) -> axum::response::Response {
    let select = &parsed.select;
    let query_type = query_type(parsed);

    let output = match run_analyze_query(state, parsed, &req.params) {
        Ok(o) => o,
        Err(resp) => return *resp,
    };

    // Single-sourced from core: steps, features and cost all derive from
    // the executed plan (#2678).
    let plan = core_plan_steps(&output.plan);
    let features = ExplainFeatures::from_plan(&output.plan);
    let estimated_cost = ExplainCost::from_plan(&output.plan);

    let (actual_stats_resp, actual_time, node_stats_resp) = extract_analyze_stats(&output);

    Json(ExplainResponse {
        query: req.query.clone(),
        query_type: query_type.to_string(),
        collection: select.from.clone(),
        plan,
        estimated_cost,
        features,
        cache_hit: output.plan.cache_hit,
        plan_reuse_count: output.plan.plan_reuse_count,
        estimated_cost_ms: Some(output.plan.estimated_cost_ms),
        actual_time_ms: actual_time,
        actual_stats: actual_stats_resp,
        node_stats: node_stats_resp,
    })
    .into_response()
}

/// Runs the core `explain_analyze_query` call, mapping core errors to the
/// matching HTTP responses. Returns `Ok(output)` on success or `Err(response)`
/// with the error already rendered.
fn run_analyze_query(
    state: &AppState,
    parsed: &velesdb_core::velesql::Query,
    params: &std::collections::HashMap<String, serde_json::Value>,
) -> std::result::Result<velesdb_core::velesql::ExplainOutput, Box<axum::response::Response>> {
    match state.db.explain_analyze_query(parsed, params) {
        Ok(o) => Ok(o),
        Err(CoreError::CollectionNotFound(name)) => {
            Err(Box::new(velesql_collection_not_found(&name)))
        }
        Err(e) => Err(Box::new(velesql_execution_error(
            &e,
            "VELESQL_EXPLAIN_ANALYZE_ERROR",
            "Validate query semantics and parameter types against the target collection",
        ))),
    }
}

/// Splits the optional `actual_stats` + `node_stats` of an EXPLAIN ANALYZE
/// output into the three response-side options consumed by
/// [`ExplainResponse`].
fn extract_analyze_stats(
    output: &velesdb_core::velesql::ExplainOutput,
) -> (
    Option<ActualStatsResponse>,
    Option<f64>,
    Option<Vec<NodeStatsResponse>>,
) {
    let Some(ref stats) = output.actual_stats else {
        return (None, None, None);
    };
    let ns: Vec<NodeStatsResponse> = output
        .node_stats
        .iter()
        .map(NodeStatsResponse::from)
        .collect();
    (
        Some(ActualStatsResponse::from(stats)),
        Some(stats.actual_time_ms),
        Some(ns),
    )
}

/// Maps a core [`QueryPlan`] into the REST [`ExplainStep`] list.
///
/// The plan steps are single-sourced from `velesdb-core` (`to_plan_steps`),
/// so the server no longer reconstructs them from the parsed AST.
fn core_plan_steps(plan: &QueryPlan) -> Vec<ExplainStep> {
    plan.to_plan_steps().iter().map(ExplainStep::from).collect()
}
