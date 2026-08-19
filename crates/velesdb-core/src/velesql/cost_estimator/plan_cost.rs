//! Plan-tree cost estimation (issue #471).
//!
//! Extracted from the monolithic `cost_estimator.rs` to respect the 500 NLOC
//! file limit (Devin Finding F on PR #606). Implements
//! [`CostEstimator::estimate_plan_cost`] and the per-node cost helpers that
//! walk a [`PlanNode`] tree and aggregate its execution cost using the
//! calibrated [`OperationCostFactors`].

use super::{default_factors, Cost, CostEstimator};
use crate::velesql::explain::{IndexLookupPlan, MatchTraversalPlan, PlanNode, VectorSearchPlan};

impl CostEstimator<'_> {
    /// Estimates the total cost of executing a plan tree.
    ///
    /// Walks the plan recursively and dispatches each node to the appropriate
    /// per-node cost function. `Sequence` nodes sum their children's costs.
    ///
    /// Returns a [`Cost`] whose `total()` can be converted to milliseconds by
    /// the caller using a `COST_UNIT_TO_MS` constant.
    #[must_use]
    pub fn estimate_plan_cost(&self, root: &PlanNode) -> Cost {
        match root {
            PlanNode::VectorSearch(vs) => self.estimate_vector_search_node_cost(vs),
            PlanNode::Filter(f) => self.estimate_filter_cost_from_selectivity(f.selectivity),
            PlanNode::TableScan(_) => self.estimate_table_scan_cost(),
            PlanNode::IndexLookup(plan) => self.estimate_index_lookup_cost(plan),
            PlanNode::MatchTraversal(mt) => self.estimate_match_traversal_cost(mt),
            PlanNode::Sequence(nodes) => nodes.iter().fold(Cost::default(), |acc, n| {
                let c = self.estimate_plan_cost(n);
                Cost::new(acc.io_cost + c.io_cost, acc.cpu_cost + c.cpu_cost)
            }),
            PlanNode::Limit(_)
            | PlanNode::Offset(_)
            | PlanNode::Join(_)
            | PlanNode::GroupBy(_)
            | PlanNode::Aggregate(_)
            | PlanNode::Sort(_) => self.estimate_limit_offset_cost(),
        }
    }

    /// Cost of a vector search node, scaling with `ef_search` and candidates.
    ///
    /// Uses the same `(ef + k) * log2(total)` probe formula as the public
    /// HNSW cost helpers and delegates the probe → Cost conversion to
    /// [`CostEstimator::hnsw_cost_from_probe`], so a future change to the
    /// HNSW factor-ratio model updates all three call sites at once.
    fn estimate_vector_search_node_cost(&self, vs: &VectorSearchPlan) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;
        let ef = f64::from(vs.ef_search.max(1));
        let k = f64::from(vs.candidates.max(1));
        // HNSW probe count scales with ef_search (frontier size) and k (results).
        // log2(total) captures the graph-height component.
        let probe = (ef + k) * total.log2().max(1.0);
        self.hnsw_cost_from_probe(probe)
    }

    /// Cost of a full table scan, proportional to row count.
    fn estimate_table_scan_cost(&self) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;

        let f = self.factors();
        let d = default_factors();
        let io_ratio = f.seq_page_cost / d.seq_page_cost;
        let cpu_ratio = f.cpu_tuple_cost / d.cpu_tuple_cost;

        // Full scan = every row paid at sequential-read + tuple-processing cost.
        Cost::new(total * io_ratio, total * cpu_ratio)
    }

    /// Cost of a property index lookup — O(log n) with a low multiplicative
    /// constant. Always cheaper than a filter or scan over the same rows.
    ///
    /// When per-column statistics are available, scales with the distinct-value
    /// count (NDV) of the indexed property rather than the full collection
    /// size — a high-cardinality index probe is cheaper than a low-cardinality
    /// one because the B-tree height tracks NDV, not row count (issue #607).
    fn estimate_index_lookup_cost(&self, plan: &IndexLookupPlan) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;

        // Prefer per-column NDV when ANALYZE has populated column_stats for
        // this property. Falls back to log2(total) when stats are unavailable
        // so the heuristic path stays bit-for-bit compatible.
        let probe_size = self
            .stats
            .column_stats
            .get(&plan.property)
            .map_or(total, |cs| {
                let ndv = cs.distinct_values.max(cs.distinct_count);
                if ndv > 0 {
                    (ndv as f64).max(1.0)
                } else {
                    total
                }
            });
        let log_probe = probe_size.log2().max(1.0);

        let f = self.factors();
        let d = default_factors();
        let cpu_ratio = f.cpu_index_cost / d.cpu_index_cost;

        // Use cpu_index_cost * log2(probe_size); negligible I/O because
        // property indexes are typically resident in memory.
        Cost::new(0.0, log_probe * cpu_ratio * d.cpu_index_cost)
    }

    /// Cost of a MATCH traversal, scaling exponentially with depth and
    /// average graph degree — the canonical BFS frontier formula.
    fn estimate_match_traversal_cost(&self, mt: &MatchTraversalPlan) -> Cost {
        // Approximate traversal fan-out: assume average degree ≈ 4 when the
        // core CollectionStats has no graph info; a future wiring will plug
        // `match_planner::CollectionStats::avg_degree` through this path.
        let avg_degree: f64 = 4.0;
        let depth = f64::from(mt.max_depth.max(1));
        // Frontier ≈ avg_degree^depth (geometric expansion), capped to total.
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;
        let frontier = avg_degree.powf(depth).min(total);

        let f = self.factors();
        let d = default_factors();
        let edge_ratio = f.cpu_edge_cost / d.cpu_edge_cost;

        Cost::new(0.0, frontier * edge_ratio * d.cpu_edge_cost)
    }

    /// Cost of a Limit or Offset node — proportional to tuples passing through,
    /// using the configured `cpu_tuple_cost`. Negligible but non-zero so that
    /// plans with many pipeline stages are penalised.
    fn estimate_limit_offset_cost(&self) -> Cost {
        let f = self.factors();
        let d = default_factors();
        let cpu_ratio = f.cpu_tuple_cost / d.cpu_tuple_cost;
        // Treat Limit/Offset as traversing a handful of rows; the real count
        // is known by the caller but is a second-order effect on total cost.
        Cost::new(0.0, d.cpu_tuple_cost * cpu_ratio)
    }
}

#[cfg(test)]
#[path = "plan_cost_tests.rs"]
mod tests;
