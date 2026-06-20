//! EXPLAIN query handler and plan building logic.

use axum::{extract::State, response::IntoResponse, Json};
use std::sync::Arc;
use velesdb_core::velesql::{Condition, QueryPlan, SelectColumns};

use crate::types::{
    ActualStatsResponse, ExplainCost, ExplainFeatures, ExplainRequest, ExplainResponse,
    ExplainStep, NodeStatsResponse,
};
use crate::AppState;

use super::velesql_helpers::{parse_and_validate, velesql_collection_not_found, velesql_error};
use axum::http::StatusCode;
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
        (status = 400, description = "Query syntax error", body = crate::types::QueryErrorResponse),
        (status = 422, description = "Query validation/execution error", body = crate::types::VelesqlErrorResponse),
        (status = 404, description = "Collection not found", body = crate::types::VelesqlErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn explain(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExplainRequest>,
) -> impl IntoResponse {
    let parsed = match parse_and_validate(&req.query) {
        Ok(q) => q,
        Err(resp) => return resp,
    };

    let select = &parsed.select;

    let collection_exists = state.db.get_any_collection(&select.from).is_some();
    if !collection_exists && !select.from.is_empty() {
        return velesql_collection_not_found(&select.from);
    }

    if req.analyze {
        return explain_with_analyze(&state, &req, &parsed);
    }

    explain_plan_only(&state, &req, &parsed)
}

/// Computes the EXPLAIN preamble shared by the plan-only and ANALYZE paths:
/// detected query features, estimated cost, and the query-type label.
fn explain_preamble(
    parsed: &velesdb_core::velesql::Query,
) -> (ExplainFeatures, ExplainCost, &'static str) {
    let features = detect_explain_features(&parsed.select);
    let estimated_cost = estimate_cost(features.has_vector_search);
    let query_type = if parsed.is_match_query() {
        "MATCH"
    } else {
        "SELECT"
    };
    (features, estimated_cost, query_type)
}

/// Build an EXPLAIN-only response (no execution).
fn explain_plan_only(
    state: &AppState,
    req: &ExplainRequest,
    parsed: &velesdb_core::velesql::Query,
) -> axum::response::Response {
    let select = &parsed.select;
    let (features, estimated_cost, query_type) = explain_preamble(parsed);

    // Single-sourced from core: the plan steps come from the canonical
    // `QueryPlan`, not a server-side AST reconstruction. The DB-less fallback
    // keeps EXPLAIN working when the collection cannot be resolved.
    let (plan, cache_hit, plan_reuse_count) = match state.db.explain_query(parsed) {
        Ok(qp) => (core_plan_steps(&qp), qp.cache_hit, qp.plan_reuse_count),
        Err(_) => (core_plan_steps(&QueryPlan::from_query(parsed)), None, None),
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
    let (features, estimated_cost, query_type) = explain_preamble(parsed);

    let output = match run_analyze_query(state, parsed, &req.params) {
        Ok(o) => o,
        Err(resp) => return *resp,
    };

    // Single-sourced from core: steps derive from the executed plan.
    let plan = core_plan_steps(&output.plan);

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
        Err(e) => Err(Box::new(velesql_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VELESQL_EXPLAIN_ANALYZE_ERROR",
            &e.to_string(),
            "Validate query semantics and parameter types against the target collection",
            None,
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

/// Detect query features from a SELECT statement for EXPLAIN output.
fn detect_explain_features(select: &velesdb_core::velesql::SelectStatement) -> ExplainFeatures {
    let has_vector_search = select
        .where_clause
        .as_ref()
        .map(condition_has_vector_search)
        .unwrap_or(false);

    ExplainFeatures {
        has_vector_search,
        has_filter: select.where_clause.is_some() && !has_vector_search,
        has_order_by: select.order_by.is_some(),
        has_group_by: select.group_by.is_some(),
        has_aggregation: match &select.columns {
            SelectColumns::Aggregations(_) => true,
            SelectColumns::Mixed { aggregations, .. } => !aggregations.is_empty(),
            _ => false,
        },
        has_join: !select.joins.is_empty(),
        has_fusion: select.fusion_clause.is_some(),
        limit: select.limit,
        offset: select.offset,
    }
}

/// Maps a core [`QueryPlan`] into the REST [`ExplainStep`] list.
///
/// The plan steps are single-sourced from `velesdb-core` (`to_plan_steps`),
/// so the server no longer reconstructs them from the parsed AST.
fn core_plan_steps(plan: &QueryPlan) -> Vec<ExplainStep> {
    plan.to_plan_steps().iter().map(ExplainStep::from).collect()
}

/// Estimate execution cost based on query features.
fn estimate_cost(has_vector_search: bool) -> ExplainCost {
    ExplainCost {
        uses_index: has_vector_search,
        index_name: if has_vector_search {
            Some("HNSW".to_string())
        } else {
            None
        },
        selectivity: if has_vector_search { 0.01 } else { 1.0 },
        complexity: if has_vector_search {
            "O(log n)"
        } else {
            "O(n)"
        }
        .to_string(),
    }
}

/// Check if a condition contains vector search.
pub(super) fn condition_has_vector_search(cond: &Condition) -> bool {
    match cond {
        Condition::VectorSearch(_)
        | Condition::VectorFusedSearch { .. }
        | Condition::SparseVectorSearch(_)
        | Condition::Similarity(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            condition_has_vector_search(left) || condition_has_vector_search(right)
        }
        Condition::Group(inner) | Condition::Not(inner) => condition_has_vector_search(inner),
        _ => false,
    }
}
