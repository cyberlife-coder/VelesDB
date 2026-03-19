//! EXPLAIN query handler and plan building logic.

use axum::{extract::State, response::IntoResponse, Json};
use std::sync::Arc;
use velesdb_core::velesql::{Condition, SelectColumns};

use crate::types::{ExplainCost, ExplainFeatures, ExplainRequest, ExplainResponse, ExplainStep};
use crate::AppState;

use super::velesql_helpers::{parse_and_validate, velesql_collection_not_found};

/// Explain a VelesQL query without executing it (EPIC-058 US-002).
///
/// Returns the query plan, estimated costs, and detected features.
#[utoipa::path(
    post,
    path = "/query/explain",
    tag = "query",
    request_body = ExplainRequest,
    responses(
        (status = 200, description = "Query plan", body = ExplainResponse),
        (status = 400, description = "Query syntax error", body = crate::types::QueryErrorResponse),
        (status = 404, description = "Collection not found", body = crate::types::VelesqlErrorResponse)
    )
)]
#[allow(clippy::unused_async, deprecated)]
pub async fn explain(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExplainRequest>,
) -> impl IntoResponse {
    let parsed = match parse_and_validate(&req.query) {
        Ok(q) => q,
        Err(resp) => return resp,
    };

    let select = &parsed.select;

    let collection_exists = state.db.get_collection(&select.from).is_some();
    if !collection_exists && !select.from.is_empty() {
        return velesql_collection_not_found(&select.from);
    }

    let features = detect_explain_features(select);
    let plan = build_explain_plan(select, &features);
    let estimated_cost = estimate_cost(features.has_vector_search);

    let query_type = if parsed.is_match_query() {
        "MATCH"
    } else {
        "SELECT"
    };

    let (cache_hit, plan_reuse_count) = state
        .db
        .explain_query(&parsed)
        .ok()
        .map_or((None, None), |qp| (qp.cache_hit, qp.plan_reuse_count));

    Json(ExplainResponse {
        query: req.query,
        query_type: query_type.to_string(),
        collection: select.from.clone(),
        plan,
        estimated_cost,
        features,
        cache_hit,
        plan_reuse_count,
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
        }
    } else {
        ExplainStep {
            step: step_num,
            operation: "FullScan".to_string(),
            description: format!("Scan collection '{}'", select.from),
            estimated_rows: None,
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
        });
        *step_num += 1;
    }

    for join in &select.joins {
        plan.push(ExplainStep {
            step: *step_num,
            operation: format!("{:?}Join", join.join_type),
            description: format!("Join with '{}'", join.table),
            estimated_rows: None,
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
        });
        *step_num += 1;
    }

    if features.has_aggregation {
        plan.push(ExplainStep {
            step: *step_num,
            operation: "Aggregate".to_string(),
            description: "Compute aggregate functions (COUNT, SUM, etc.)".to_string(),
            estimated_rows: None,
        });
        *step_num += 1;
    }

    if features.has_order_by {
        plan.push(ExplainStep {
            step: *step_num,
            operation: "Sort".to_string(),
            description: "Sort results by ORDER BY clause".to_string(),
            estimated_rows: None,
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
