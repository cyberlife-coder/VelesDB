//! Plan-tree cost estimation (issue #471).
//!
//! Extracted from the monolithic `cost_estimator.rs` to respect the 500 NLOC
//! file limit (Devin Finding F on PR #606). Implements
//! [`CostEstimator::estimate_plan_cost`] and the per-node cost helpers that
//! walk a [`PlanNode`] tree and aggregate its execution cost using the
//! calibrated [`OperationCostFactors`].

use super::{default_factors, Cost, CostEstimator};
use crate::velesql::explain::{MatchTraversalPlan, PlanNode, VectorSearchPlan};

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
            PlanNode::IndexLookup(_) => self.estimate_index_lookup_cost(),
            PlanNode::MatchTraversal(mt) => self.estimate_match_traversal_cost(mt),
            PlanNode::Sequence(nodes) => nodes.iter().fold(Cost::default(), |acc, n| {
                let c = self.estimate_plan_cost(n);
                Cost::new(acc.io_cost + c.io_cost, acc.cpu_cost + c.cpu_cost)
            }),
            PlanNode::Limit(_) | PlanNode::Offset(_) => self.estimate_limit_offset_cost(),
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
    fn estimate_index_lookup_cost(&self) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;
        let log_probe = total.log2().max(1.0);

        let f = self.factors();
        let d = default_factors();
        let cpu_ratio = f.cpu_index_cost / d.cpu_index_cost;

        // Use cpu_index_cost * log2(total); negligible I/O because property
        // indexes are typically resident in memory.
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
mod tests {
    //! Unit tests for the `estimate_plan_cost` API. These exercise the
    //! cost-monotonicity invariants independently of the EXPLAIN pipeline.

    use super::*;
    use crate::collection::stats::CollectionStats;
    use crate::velesql::explain::{
        FilterPlan, IndexLookupPlan, LimitPlan, MatchTraversalPlan, PlanNode, TableScanPlan,
        VectorSearchPlan,
    };

    /// Builds a `CollectionStats` with a fixed total point count.
    fn stats_with_points(total: u64) -> CollectionStats {
        let mut s = CollectionStats::new();
        s.total_points = total;
        s.row_count = total;
        s
    }

    #[test]
    fn plan_cost_vector_search_scales_with_ef_search() {
        let stats = stats_with_points(10_000);
        let est = CostEstimator::new(&stats);

        let low_ef = PlanNode::VectorSearch(VectorSearchPlan {
            collection: "t".into(),
            ef_search: 50,
            candidates: 10,
        });
        let high_ef = PlanNode::VectorSearch(VectorSearchPlan {
            collection: "t".into(),
            ef_search: 500,
            candidates: 10,
        });

        let c_low = est.estimate_plan_cost(&low_ef).total();
        let c_high = est.estimate_plan_cost(&high_ef).total();
        assert!(
            c_high > c_low,
            "larger ef_search must cost more: low={c_low} high={c_high}"
        );
    }

    #[test]
    fn plan_cost_table_scan_scales_with_collection_size() {
        let small = stats_with_points(100);
        let large = stats_with_points(10_000);

        let scan = PlanNode::TableScan(TableScanPlan {
            collection: "t".into(),
        });

        let c_small = CostEstimator::new(&small).estimate_plan_cost(&scan).total();
        let c_large = CostEstimator::new(&large).estimate_plan_cost(&scan).total();
        assert!(
            c_large > c_small,
            "larger collection must cost more to scan: small={c_small} large={c_large}"
        );
    }

    #[test]
    fn plan_cost_index_lookup_cheaper_than_table_scan() {
        let stats = stats_with_points(100_000);
        let est = CostEstimator::new(&stats);

        let scan = PlanNode::TableScan(TableScanPlan {
            collection: "t".into(),
        });
        let lookup = PlanNode::IndexLookup(IndexLookupPlan {
            label: "t".into(),
            property: "id".into(),
            value: "1".into(),
        });

        let c_scan = est.estimate_plan_cost(&scan).total();
        let c_lookup = est.estimate_plan_cost(&lookup).total();
        assert!(
            c_lookup < c_scan,
            "index lookup must be cheaper than full scan: lookup={c_lookup} scan={c_scan}"
        );
    }

    #[test]
    fn plan_cost_match_traversal_scales_with_depth() {
        let stats = stats_with_points(1_000);
        let est = CostEstimator::new(&stats);

        let shallow = PlanNode::MatchTraversal(MatchTraversalPlan {
            strategy: "graph-first".into(),
            start_labels: vec!["A".into()],
            max_depth: 1,
            relationship_count: 1,
            has_similarity: false,
            similarity_threshold: None,
        });
        let deep = PlanNode::MatchTraversal(MatchTraversalPlan {
            strategy: "graph-first".into(),
            start_labels: vec!["A".into()],
            max_depth: 3,
            relationship_count: 1,
            has_similarity: false,
            similarity_threshold: None,
        });

        let c_shallow = est.estimate_plan_cost(&shallow).total();
        let c_deep = est.estimate_plan_cost(&deep).total();
        assert!(
            c_deep > c_shallow,
            "deeper traversal must cost more: shallow={c_shallow} deep={c_deep}"
        );
    }

    #[test]
    fn plan_cost_sequence_sums_children() {
        let stats = stats_with_points(1_000);
        let est = CostEstimator::new(&stats);

        let scan = PlanNode::TableScan(TableScanPlan {
            collection: "t".into(),
        });
        let filter = PlanNode::Filter(FilterPlan {
            conditions: "x = 1".into(),
            selectivity: 0.1,
            estimated_rows: None,
            estimation_method: None,
        });
        let limit = PlanNode::Limit(LimitPlan { count: 10 });

        let c_scan = est.estimate_plan_cost(&scan).total();
        let c_filter = est.estimate_plan_cost(&filter).total();
        let c_limit = est.estimate_plan_cost(&limit).total();

        let sequence = PlanNode::Sequence(vec![scan, filter, limit]);
        let c_seq = est.estimate_plan_cost(&sequence).total();

        let expected = c_scan + c_filter + c_limit;
        assert!(
            (c_seq - expected).abs() < 1e-9,
            "Sequence cost must equal sum of child costs: seq={c_seq} expected={expected}"
        );
    }

    #[test]
    fn plan_cost_filter_from_selectivity_monotone() {
        let stats = stats_with_points(10_000);
        let est = CostEstimator::new(&stats);

        let low_sel = est.estimate_filter_cost_from_selectivity(0.01).total();
        let high_sel = est.estimate_filter_cost_from_selectivity(0.5).total();
        assert!(
            high_sel > low_sel,
            "higher selectivity means more rows scanned → higher cost"
        );
    }

    #[test]
    fn plan_cost_empty_stats_does_not_panic() {
        // Regression guard: corrupt-looking stats (zero points, no histogram)
        // must still produce a finite cost via the `.max(1)` floors.
        let stats = CollectionStats::new();
        let est = CostEstimator::new(&stats);

        let plan = PlanNode::VectorSearch(VectorSearchPlan {
            collection: "t".into(),
            ef_search: 100,
            candidates: 10,
        });
        let cost = est.estimate_plan_cost(&plan).total();
        assert!(cost.is_finite() && cost > 0.0);
    }

    #[test]
    fn hnsw_cost_on_size_scales_logarithmically() {
        // Devin finding E on #606: the reduced-set HNSW cost must scale with
        // log2(size), not linearly. Doubling the size increases the cost by
        // exactly one probe "step" of (ef + k).
        let stats = stats_with_points(100_000);
        let est = CostEstimator::new(&stats);

        let small = est
            .estimate_hnsw_search_cost_with_ef_on_size(100, 10, 1_000)
            .total();
        let big = est
            .estimate_hnsw_search_cost_with_ef_on_size(100, 10, 1_000_000)
            .total();

        assert!(
            big > small,
            "cost must grow with collection size: small={small} big={big}"
        );
        // Logarithmic (not linear) scaling: going from 1K to 1M rows
        // multiplies log2 by 2.0 (from ~10 to ~20), so the ratio must stay
        // well below 1000× (= the linear scaling result).
        assert!(
            big / small < 5.0,
            "HNSW cost must scale logarithmically (ratio < 5), got {}",
            big / small
        );
    }

    #[test]
    fn hnsw_cost_on_full_size_matches_default_variant() {
        // Backward-compat: `estimate_hnsw_search_cost_with_ef` must return
        // exactly the same cost as
        // `estimate_hnsw_search_cost_with_ef_on_size(stats.total_points)`.
        let stats = stats_with_points(42_000);
        let est = CostEstimator::new(&stats);

        let implicit = est.estimate_hnsw_search_cost_with_ef(100, 10).total();
        let explicit = est
            .estimate_hnsw_search_cost_with_ef_on_size(100, 10, 42_000)
            .total();

        assert!(
            (implicit - explicit).abs() < f64::EPSILON,
            "the two variants must produce identical costs when called with \
             the full collection size: implicit={implicit} explicit={explicit}"
        );
    }
}
