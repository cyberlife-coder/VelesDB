//! EXPLAIN query handler and plan building logic.

use axum::{extract::State, response::IntoResponse, Json};
use std::sync::Arc;
use velesdb_core::velesql::{Condition, SelectColumns};

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

/// Build an EXPLAIN-only response (no execution).
fn explain_plan_only(
    state: &AppState,
    req: &ExplainRequest,
    parsed: &velesdb_core::velesql::Query,
) -> axum::response::Response {
    let select = &parsed.select;
    let features = detect_explain_features(select);
    let mut plan = build_explain_plan(select, &features);
    let estimated_cost = estimate_cost(features.has_vector_search);
    let query_type = if parsed.is_match_query() {
        "MATCH"
    } else {
        "SELECT"
    };

    let (cache_hit, plan_reuse_count) =
        state
            .db
            .explain_query(parsed)
            .ok()
            .map_or((None, None), |qp| {
                merge_core_estimation(&mut plan, &qp);
                (qp.cache_hit, qp.plan_reuse_count)
            });

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
    let features = detect_explain_features(select);
    let mut plan = build_explain_plan(select, &features);
    let estimated_cost = estimate_cost(features.has_vector_search);
    let query_type = if parsed.is_match_query() {
        "MATCH"
    } else {
        "SELECT"
    };

    let output = match state.db.explain_analyze_query(parsed, &req.params) {
        Ok(o) => o,
        Err(CoreError::CollectionNotFound(name)) => {
            return velesql_collection_not_found(&name);
        }
        Err(e) => {
            return velesql_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "VELESQL_EXPLAIN_ANALYZE_ERROR",
                &e.to_string(),
                "Validate query semantics and parameter types against the target collection",
                None,
            );
        }
    };

    // Merge core estimation metadata into server plan (graceful: already have output).
    merge_core_estimation(&mut plan, &output.plan);

    let (actual_stats_resp, actual_time, node_stats_resp) =
        if let Some(ref stats) = output.actual_stats {
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
        } else {
            (None, None, None)
        };

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

/// Build the execution plan steps for an EXPLAIN response.
fn build_explain_plan(
    select: &velesdb_core::velesql::SelectStatement,
    features: &ExplainFeatures,
) -> Vec<ExplainStep> {
    let mut plan = Vec::new();
    let mut step_num = 1;

    plan.push(build_source_step(select, features, step_num));
    step_num += 1;

    append_filter_and_join_steps(select, features, &mut plan, &mut step_num);
    append_aggregation_steps(features, &mut plan, &mut step_num);
    append_pagination_step(select, &mut plan, step_num);

    plan
}

fn build_source_step(
    select: &velesdb_core::velesql::SelectStatement,
    features: &ExplainFeatures,
    step_num: usize,
) -> ExplainStep {
    if features.has_vector_search {
        ExplainStep {
            step: step_num,
            operation: "VectorSearch".to_string(),
            description: "ANN search using HNSW index with NEAR clause".to_string(),
            estimated_rows: select.limit.map(|l| l as usize),
            estimation_method: None,
        }
    } else {
        ExplainStep {
            step: step_num,
            operation: "FullScan".to_string(),
            description: format!("Scan collection '{}'", select.from),
            estimated_rows: None,
            estimation_method: None,
        }
    }
}

fn append_filter_and_join_steps(
    select: &velesdb_core::velesql::SelectStatement,
    features: &ExplainFeatures,
    plan: &mut Vec<ExplainStep>,
    step_num: &mut usize,
) {
    if features.has_filter {
        plan.push(ExplainStep {
            step: *step_num,
            operation: "Filter".to_string(),
            description: "Apply WHERE clause predicates".to_string(),
            estimated_rows: None,
            estimation_method: None,
        });
        *step_num += 1;
    }

    for join in &select.joins {
        plan.push(ExplainStep {
            step: *step_num,
            operation: format!("{:?}Join", join.join_type),
            description: format!("Join with '{}'", join.table),
            estimated_rows: None,
            estimation_method: None,
        });
        *step_num += 1;
    }
}

fn append_aggregation_steps(
    features: &ExplainFeatures,
    plan: &mut Vec<ExplainStep>,
    step_num: &mut usize,
) {
    if features.has_group_by {
        plan.push(ExplainStep {
            step: *step_num,
            operation: "GroupBy".to_string(),
            description: "Group rows by specified columns".to_string(),
            estimated_rows: None,
            estimation_method: None,
        });
        *step_num += 1;
    }

    if features.has_aggregation {
        plan.push(ExplainStep {
            step: *step_num,
            operation: "Aggregate".to_string(),
            description: "Compute aggregate functions (COUNT, SUM, etc.)".to_string(),
            estimated_rows: None,
            estimation_method: None,
        });
        *step_num += 1;
    }

    if features.has_order_by {
        plan.push(ExplainStep {
            step: *step_num,
            operation: "Sort".to_string(),
            description: "Sort results by ORDER BY clause".to_string(),
            estimated_rows: None,
            estimation_method: None,
        });
        *step_num += 1;
    }
}

fn append_pagination_step(
    select: &velesdb_core::velesql::SelectStatement,
    plan: &mut Vec<ExplainStep>,
    step_num: usize,
) {
    if select.limit.is_some() || select.offset.is_some() {
        plan.push(ExplainStep {
            step: step_num,
            operation: "Limit".to_string(),
            description: format!(
                "Apply LIMIT {} OFFSET {}",
                select.limit.unwrap_or(0),
                select.offset.unwrap_or(0)
            ),
            estimated_rows: select.limit.map(|l| l as usize),
            estimation_method: None,
        });
    }
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
        | Condition::Similarity(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            condition_has_vector_search(left) || condition_has_vector_search(right)
        }
        Condition::Group(inner) | Condition::Not(inner) => condition_has_vector_search(inner),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Core plan merge helpers (Task 2.2)
// ---------------------------------------------------------------------------

/// Recursively extracts the first `FilterPlan` from a core `PlanNode` tree.
fn extract_filter_plan(
    node: &velesdb_core::velesql::PlanNode,
) -> Option<&velesdb_core::velesql::FilterPlan> {
    match node {
        velesdb_core::velesql::PlanNode::Filter(fp) => Some(fp),
        velesdb_core::velesql::PlanNode::Sequence(nodes) => {
            nodes.iter().find_map(extract_filter_plan)
        }
        _ => None,
    }
}

/// Merges core `FilterPlan` estimation data into server `ExplainStep` entries.
///
/// Copies `estimated_rows` and `estimation_method` from the core plan's
/// `FilterPlan` into every server step whose `operation` is `"Filter"`.
#[allow(clippy::cast_possible_truncation)]
fn merge_core_estimation(plan: &mut [ExplainStep], core_plan: &velesdb_core::velesql::QueryPlan) {
    if let Some(fp) = extract_filter_plan(&core_plan.root) {
        for step in plan.iter_mut() {
            if step.operation == "Filter" {
                step.estimated_rows = fp.estimated_rows.map(|r| r as usize);
                step.estimation_method = fp.estimation_method.clone();
            }
        }
    }
}
