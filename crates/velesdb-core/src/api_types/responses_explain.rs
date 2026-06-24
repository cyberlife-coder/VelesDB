//! EXPLAIN-related response DTOs for the `VelesDB` API.

use serde::Serialize;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ============================================================================
// EXPLAIN Responses
// ============================================================================

/// Response from query EXPLAIN.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainResponse {
    /// The original query.
    pub query: String,
    /// Query type (SELECT, MATCH, etc.).
    pub query_type: String,
    /// Target collection name.
    pub collection: String,
    /// Query plan steps.
    pub plan: Vec<ExplainStep>,
    /// Estimated cost metrics.
    pub estimated_cost: ExplainCost,
    /// Query features detected.
    pub features: ExplainFeatures,
    /// Whether this plan was served from the compiled plan cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub cache_hit: Option<bool>,
    /// How many times this cached plan has been reused.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub plan_reuse_count: Option<u64>,
    /// Estimated cost in milliseconds (for side-by-side comparison with actual).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub estimated_cost_ms: Option<f64>,
    /// Actual execution time in milliseconds (only when `analyze: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub actual_time_ms: Option<f64>,
    /// Actual execution statistics (only when `analyze: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub actual_stats: Option<ActualStatsResponse>,
    /// Per-node execution statistics (only when `analyze: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub node_stats: Option<Vec<NodeStatsResponse>>,
}

/// Actual execution statistics for EXPLAIN ANALYZE responses.
///
/// `actual_rows`, `actual_time_ms`, and `loops` are measured. The graph
/// traversal counters `nodes_visited` / `edges_traversed` are **strategy-
/// dependent approximations** (a lower bound, not an exact figure):
/// `VectorFirst` undercounts via its `limit(1)` existence-BFS frontier and
/// `Parallel` double-counts a node touched by both legs. The
/// `traversal_counters_approximate` flag exposes this honesty contract in a
/// machine-readable form (backlog #26), mirroring [`NodeStatsResponse::estimated`].
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActualStatsResponse {
    /// Actual number of rows returned.
    pub actual_rows: u64,
    /// Actual execution time in milliseconds.
    pub actual_time_ms: f64,
    /// Number of loop iterations.
    pub loops: u64,
    /// Approximate number of nodes visited during MATCH graph traversal.
    /// See `traversal_counters_approximate`; 0 for non-graph queries.
    pub nodes_visited: u64,
    /// Approximate number of edges traversed during MATCH graph traversal.
    /// See `traversal_counters_approximate`; 0 for non-graph queries.
    pub edges_traversed: u64,
    /// When `true`, `nodes_visited` and `edges_traversed` are strategy-dependent
    /// approximations (a lower bound), not exact measured counts. Always `false`
    /// for non-graph queries, where both counters are 0.
    pub traversal_counters_approximate: bool,
}

#[cfg(feature = "persistence")]
impl From<&crate::velesql::ActualStats> for ActualStatsResponse {
    fn from(s: &crate::velesql::ActualStats) -> Self {
        Self {
            actual_rows: s.actual_rows,
            actual_time_ms: s.actual_time_ms,
            loops: s.loops,
            nodes_visited: s.nodes_visited,
            edges_traversed: s.edges_traversed,
            // Graph traversal counters are only present (and only approximate)
            // when a MATCH query actually walked the graph (backlog #26).
            traversal_counters_approximate: s.nodes_visited > 0 || s.edges_traversed > 0,
        }
    }
}

/// Per-plan-node **estimated** execution statistics for EXPLAIN ANALYZE responses.
///
/// All values are synthetic heuristics derived from the plan-global
/// `actual_time_ms` — they are **not** individually measured per node.
/// Field names keep the `actual_` prefix for API stability; check the
/// `estimated` flag to distinguish heuristic values from future
/// instrumented measurements (#467).
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct NodeStatsResponse {
    /// Node label (e.g. `VectorSearch`, `Filter`, `Limit`).
    pub node_label: String,
    /// Estimated wall-clock time for this node in milliseconds.
    /// Derived from total execution time using normalized weight fractions,
    /// not real per-node instrumentation. Will be replaced by measured
    /// timing (#467).
    pub actual_time_ms: f64,
    /// Estimated rows entering this node (heuristic approximation).
    pub actual_rows_in: u64,
    /// Estimated rows leaving this node (heuristic approximation).
    pub actual_rows_out: u64,
    /// Number of loop iterations (1 for non-looping nodes).
    pub loops: u64,
    /// When `true`, time and row counts are heuristic estimates, not real
    /// per-node measurements. Will become `false` once instrumented timing
    /// lands (#467).
    pub estimated: bool,
}

#[cfg(feature = "persistence")]
impl From<&crate::velesql::NodeStats> for NodeStatsResponse {
    fn from(ns: &crate::velesql::NodeStats) -> Self {
        Self {
            node_label: ns.node_label.clone(),
            actual_time_ms: ns.actual_time_ms,
            actual_rows_in: ns.actual_rows_in,
            actual_rows_out: ns.actual_rows_out,
            loops: ns.loops,
            estimated: ns.estimated,
        }
    }
}

/// A step in the query execution plan.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainStep {
    /// Step number (1-indexed).
    pub step: usize,
    /// Operation type.
    pub operation: String,
    /// Description of what this step does.
    pub description: String,
    /// Estimated rows processed/produced.
    pub estimated_rows: Option<usize>,
    /// How the row estimate was produced (e.g. "histogram", "cardinality", "heuristic").
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable))]
    pub estimation_method: Option<String>,
}

#[cfg(feature = "persistence")]
impl From<&crate::velesql::PlanStep> for ExplainStep {
    #[allow(clippy::cast_possible_truncation)]
    fn from(s: &crate::velesql::PlanStep) -> Self {
        Self {
            step: s.step,
            operation: s.rest_operation(),
            description: s.description.clone(),
            estimated_rows: s.estimated_rows.map(|r| r as usize),
            estimation_method: s.estimation_method.clone(),
        }
    }
}

/// Estimated cost metrics for the query.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainCost {
    /// Whether an index can be used.
    pub uses_index: bool,
    /// Index name if used.
    pub index_name: Option<String>,
    /// Estimated selectivity (0.0 - 1.0).
    pub selectivity: f64,
    /// Estimated complexity class.
    pub complexity: String,
}

/// Features detected in the query.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainFeatures {
    /// Has vector search (NEAR clause).
    pub has_vector_search: bool,
    /// Has metadata filter.
    pub has_filter: bool,
    /// Has ORDER BY clause.
    pub has_order_by: bool,
    /// Has GROUP BY clause.
    pub has_group_by: bool,
    /// Has aggregation functions.
    pub has_aggregation: bool,
    /// Has JOIN clause.
    pub has_join: bool,
    /// Has FUSION clause.
    pub has_fusion: bool,
    /// LIMIT value if present.
    pub limit: Option<u64>,
    /// OFFSET value if present.
    pub offset: Option<u64>,
}
