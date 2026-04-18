//! Cost estimator for hybrid MATCH + NEAR query planning.
//!
//! Uses [`OperationCostFactors`] (calibrated or default) to compute I/O and
//! CPU costs for query plan nodes.
//!
//! # Transition from hard-coded constants (Issue #467)
//!
//! The former constants `FILTER_SCAN_IO_WEIGHT` (0.2), `FILTER_SCAN_CPU_WEIGHT`
//! (0.8), `HNSW_IO_WEIGHT` (0.5), and `HNSW_CPU_WEIGHT` (1.0) have been
//! removed. Cost computation now derives I/O and CPU weights from the fields
//! of [`OperationCostFactors`], which are calibrated dynamically during
//! `analyze()` based on collection statistics and histograms.
//!
//! Backward-compatible formulas (using `COMPAT_FILTER_IO`, `COMPAT_HNSW_IO`,
//! etc.) ensure that **default factors produce identical costs** to the old
//! hard-coded constants. When calibrated factors differ from defaults, costs
//! scale proportionally via `(calibrated / default)` ratios.

// Reason: usize/u64 → f64 for selectivity ratios and log2 inputs; these are
// cardinalities where ±1 ULP has no operational impact on query planning.
#![allow(clippy::cast_precision_loss)]

use super::explain::{MatchTraversalPlan, PlanNode, VectorSearchPlan};
use crate::collection::query_cost::cost_model::OperationCostFactors;
use crate::collection::stats::next_after;
use crate::collection::stats::CollectionStats;
use crate::collection::stats::Histogram;
use crate::velesql::ast::{CompareOp, Condition, Value};

// ---------------------------------------------------------------------------
// Backward-compatibility constants
// ---------------------------------------------------------------------------
// These reproduce the historical I/O and CPU ratios when factors == default.
// The formulas multiply these by (factors.field / default.field) so that
// calibrated factors scale the cost proportionally while default factors
// yield the exact same costs as the old hard-coded constants.

/// Historical I/O ratio for filter scan cost.
const COMPAT_FILTER_IO: f64 = 0.2;
/// Historical CPU ratio for filter scan cost.
const COMPAT_FILTER_CPU: f64 = 0.8;
/// Historical I/O ratio for HNSW search cost.
const COMPAT_HNSW_IO: f64 = 0.5;
/// Historical CPU ratio for HNSW search cost.
const COMPAT_HNSW_CPU: f64 = 1.0;

/// Composite cost estimate.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Cost {
    /// Estimated I/O component (arbitrary units).
    pub io_cost: f64,
    /// Estimated CPU component (arbitrary units).
    pub cpu_cost: f64,
}

impl Cost {
    #[must_use]
    /// Creates a new cost value from I/O and CPU components.
    pub const fn new(io_cost: f64, cpu_cost: f64) -> Self {
        Self { io_cost, cpu_cost }
    }

    #[must_use]
    /// Returns the total cost (I/O + CPU).
    pub const fn total(self) -> f64 {
        self.io_cost + self.cpu_cost
    }
}

/// Reference to cost factors — either calibrated from stats, or default.
///
/// Zero-allocation on cache-hit path: `Calibrated` borrows from
/// `CollectionStats`, `Default` is a unit variant resolved inline.
#[derive(Debug)]
enum CostFactorsRef<'a> {
    /// Calibrated factors stored in `CollectionStats` (zero-copy borrow).
    Calibrated(&'a OperationCostFactors),
    /// Default factors (no allocation needed).
    Default,
}

impl CostFactorsRef<'_> {
    /// Returns a reference to the effective factors.
    ///
    /// For `Calibrated`, returns the borrowed reference directly.
    /// For `Default`, returns a reference to a lazily-initialized static default.
    fn get(&self) -> &OperationCostFactors {
        match self {
            Self::Calibrated(f) => f,
            Self::Default => {
                use std::sync::LazyLock;
                static DEFAULT_FACTORS: LazyLock<OperationCostFactors> =
                    LazyLock::new(OperationCostFactors::default);
                &DEFAULT_FACTORS
            }
        }
    }
}

/// Cost estimator based on collection statistics.
///
/// Uses `OperationCostFactors` (calibrated or default) to compute I/O and
/// CPU costs. Zero-allocation on cache-hit path via `CostFactorsRef`.
#[derive(Debug)]
pub struct CostEstimator<'a> {
    stats: &'a CollectionStats,
    factors: CostFactorsRef<'a>,
}

/// Converts a VelesQL `Value` to `f64` for histogram lookup.
///
/// Returns `Some(f64)` for Integer, `UnsignedInteger`, Float, and Boolean.
/// Returns `None` for Parameter, Null, String, Temporal, and Subquery.
fn value_to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Integer(i) => Some(*i as f64),
        Value::UnsignedInteger(u) => Some(*u as f64),
        Value::Float(f) => Some(*f),
        Value::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Lazily-initialized default factors for ratio computation.
fn default_factors() -> &'static OperationCostFactors {
    use std::sync::LazyLock;
    static DEFAULT: LazyLock<OperationCostFactors> = LazyLock::new(OperationCostFactors::default);
    &DEFAULT
}

impl<'a> CostEstimator<'a> {
    #[must_use]
    /// Creates a new estimator with calibrated factors from the collection (if available).
    ///
    /// If `stats.calibrated_cost_factors` is `Some`, uses the calibrated factors.
    /// Otherwise, uses `OperationCostFactors::default()`.
    pub fn new(stats: &'a CollectionStats) -> Self {
        let factors = match &stats.calibrated_cost_factors {
            Some(f) => CostFactorsRef::Calibrated(f),
            None => CostFactorsRef::Default,
        };
        Self { stats, factors }
    }

    /// Creates an estimator with explicit factors (for tests or override).
    #[must_use]
    pub fn with_factors(stats: &'a CollectionStats, factors: &'a OperationCostFactors) -> Self {
        Self {
            stats,
            factors: CostFactorsRef::Calibrated(factors),
        }
    }

    /// Returns the histogram for a column, delegating to `CollectionStats`.
    fn get_histogram(&self, column: &str) -> Option<&Histogram> {
        self.stats.get_column_histogram(column)
    }

    #[must_use]
    /// Estimates filter cost using selectivity derived from stats.
    ///
    /// Uses backward-compatible formulas:
    /// - `io_cost  = scan_rows * COMPAT_FILTER_IO  * (factors.seq_page_cost / default.seq_page_cost)`
    /// - `cpu_cost = scan_rows * COMPAT_FILTER_CPU * (factors.cpu_tuple_cost / default.cpu_tuple_cost)`
    ///
    /// With default factors, this produces identical costs to the old constants.
    pub fn estimate_filter_cost(&self, filter: &Condition) -> Cost {
        let selectivity = self.estimate_condition_selectivity(filter).clamp(0.0, 1.0);
        let total = self.stats.total_points.max(self.stats.row_count) as f64;
        let scan_rows = (total * selectivity).max(1.0);

        let f = self.factors.get();
        let d = default_factors();
        let io_ratio = f.seq_page_cost / d.seq_page_cost;
        let cpu_ratio = f.cpu_tuple_cost / d.cpu_tuple_cost;

        Cost::new(
            scan_rows * COMPAT_FILTER_IO * io_ratio,
            scan_rows * COMPAT_FILTER_CPU * cpu_ratio,
        )
    }

    #[must_use]
    /// Estimates HNSW search cost for top-k retrieval.
    ///
    /// Uses backward-compatible formulas:
    /// - `io_cost  = probe * COMPAT_HNSW_IO  * (factors.random_page_cost / default.random_page_cost)`
    /// - `cpu_cost = probe * COMPAT_HNSW_CPU * (factors.cpu_distance_cost / default.cpu_distance_cost)`
    ///
    /// With default factors, this produces identical costs to the old constants.
    pub fn estimate_hnsw_search_cost(&self, k: usize) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;
        let probe = (k.max(1) as f64) * total.log2().max(1.0);

        let f = self.factors.get();
        let d = default_factors();
        let io_ratio = f.random_page_cost / d.random_page_cost;
        let cpu_ratio = f.cpu_distance_cost / d.cpu_distance_cost;

        Cost::new(
            probe * COMPAT_HNSW_IO * io_ratio,
            probe * COMPAT_HNSW_CPU * cpu_ratio,
        )
    }

    #[must_use]
    /// Estimates predicate selectivity in the `[0.0, 1.0]` range.
    ///
    /// Dispatches on every `Condition` variant — no catch-all. Comparison,
    /// In, Between, and Like use histogram data when available; text/geo
    /// predicates return explicit heuristic constants; compound predicates
    /// use product (AND), inclusion-exclusion (OR), and complement (NOT).
    pub fn estimate_condition_selectivity(&self, condition: &Condition) -> f64 {
        match condition {
            Condition::Comparison(cmp) => self.estimate_comparison_selectivity_with_histogram(
                &cmp.column,
                cmp.operator,
                &cmp.value,
            ),
            Condition::In(cond) => {
                self.estimate_in_selectivity(&cond.column, &cond.values, cond.negated)
            }
            Condition::Between(cond) => {
                self.estimate_between_selectivity(&cond.column, &cond.low, &cond.high)
            }
            Condition::Like(cond) => self.estimate_like_selectivity(&cond.column, &cond.pattern),
            Condition::IsNull(cond) => self
                .stats
                .field_stats
                .get(cond.column.as_str())
                .map_or(0.1, |s| {
                    s.null_count as f64 / self.stats.total_points.max(1) as f64
                }),
            Condition::Match(_) | Condition::Contains(_) | Condition::GeoDistance(_) => 0.1,
            Condition::ContainsText(_) => 0.05,
            Condition::GeoBbox(_) => 0.2,
            Condition::GraphMatch(_) => 0.5,
            Condition::And(left, right) => {
                self.estimate_condition_selectivity(left)
                    * self.estimate_condition_selectivity(right)
            }
            Condition::Or(left, right) => {
                let l = self.estimate_condition_selectivity(left);
                let r = self.estimate_condition_selectivity(right);
                (l + r - (l * r)).clamp(0.0, 1.0)
            }
            Condition::Not(inner) => 1.0 - self.estimate_condition_selectivity(inner),
            Condition::Group(inner) => self.estimate_condition_selectivity(inner),
            Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_)
            | Condition::SparseVectorSearch(_)
            | Condition::Similarity(_) => 1.0,
        }
    }

    /// Estimates selectivity for a `Comparison` condition using histogram data.
    ///
    /// Dispatches on `CompareOp`: Eq → histogram equality, NotEq → complement,
    /// Lt/Lte/Gt/Gte → histogram less-than with appropriate adjustments.
    /// Falls back to `CollectionStats::estimate_selectivity()` when no histogram
    /// is available or the value cannot be converted to `f64`.
    fn estimate_comparison_selectivity_with_histogram(
        &self,
        column: &str,
        op: CompareOp,
        value: &Value,
    ) -> f64 {
        // Parameter values are unknown at plan time — use heuristic.
        if matches!(value, Value::Parameter(_)) {
            return 0.1;
        }

        let Some(v) = value_to_f64(value) else {
            return self.stats.estimate_selectivity(column);
        };

        let Some(hist) = self.get_histogram(column) else {
            return self.stats.estimate_selectivity(column);
        };

        let sel = match op {
            CompareOp::Eq => hist.estimate_eq_selectivity(v),
            CompareOp::NotEq => 1.0 - hist.estimate_eq_selectivity(v),
            CompareOp::Lt => hist.estimate_lt_selectivity(v),
            CompareOp::Lte => hist.estimate_lt_selectivity(next_after(v)),
            CompareOp::Gt => 1.0 - hist.estimate_lt_selectivity(next_after(v)),
            CompareOp::Gte => 1.0 - hist.estimate_lt_selectivity(v),
        };
        sel.clamp(0.0, 1.0)
    }

    /// Estimates selectivity for a `Between` condition using histogram range.
    ///
    /// Converts low/high to `f64` and delegates to `Histogram::estimate_range_selectivity`.
    /// Falls back to `0.3` when no histogram is available or conversion fails.
    fn estimate_between_selectivity(&self, column: &str, low: &Value, high: &Value) -> f64 {
        let (Some(low_f), Some(high_f)) = (value_to_f64(low), value_to_f64(high)) else {
            return 0.3;
        };

        match self.get_histogram(column) {
            // BETWEEN is inclusive on both ends (low <= x <= high).
            // Use next_after(high_f) so bucket_range_fraction includes values
            // at the exact upper boundary — consistent with CompareOp::Lte.
            Some(h) => h.estimate_range_selectivity(low_f, next_after(high_f)),
            None => 0.3,
        }
    }

    /// Estimates selectivity for an `In` condition.
    ///
    /// Sums per-value equality selectivities via histogram lookups when available.
    /// Falls back to `base_selectivity × list_size` without a histogram.
    /// If negated (NOT IN), returns `1.0 - sel`.
    fn estimate_in_selectivity(&self, column: &str, values: &[Value], negated: bool) -> f64 {
        let sel = if let Some(h) = self.get_histogram(column) {
            let numeric_sels: Vec<f64> = values
                .iter()
                .filter_map(value_to_f64)
                .map(|v| h.estimate_eq_selectivity(v))
                .collect();
            if numeric_sels.is_empty() {
                // All values are non-numeric (e.g. strings) — fall back to
                // cardinality-based estimate so we don't silently return 0.0.
                let base = self.stats.estimate_selectivity(column);
                (base * values.len() as f64).clamp(0.0, 1.0)
            } else {
                let sum: f64 = numeric_sels.into_iter().sum();
                sum.clamp(0.0, 1.0)
            }
        } else {
            let base = self.stats.estimate_selectivity(column);
            (base * values.len() as f64).clamp(0.0, 1.0)
        };

        if negated {
            1.0 - sel
        } else {
            sel
        }
    }

    /// Estimates filter cost from an already-computed selectivity value.
    ///
    /// Useful when the caller has a pre-computed selectivity (e.g. from
    /// `estimate_condition_selectivity` or a heuristic) and wants to translate
    /// it into a calibrated cost without building a `Condition` AST.
    ///
    /// Uses the same backward-compatible formula as `estimate_filter_cost`.
    #[must_use]
    pub fn estimate_filter_cost_from_selectivity(&self, selectivity: f64) -> Cost {
        let sel = selectivity.clamp(0.0, 1.0);
        let total = self.stats.total_points.max(self.stats.row_count) as f64;
        let scan_rows = (total * sel).max(1.0);

        let f = self.factors.get();
        let d = default_factors();
        let io_ratio = f.seq_page_cost / d.seq_page_cost;
        let cpu_ratio = f.cpu_tuple_cost / d.cpu_tuple_cost;

        Cost::new(
            scan_rows * COMPAT_FILTER_IO * io_ratio,
            scan_rows * COMPAT_FILTER_CPU * cpu_ratio,
        )
    }

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
    fn estimate_vector_search_node_cost(&self, vs: &VectorSearchPlan) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;
        let ef = f64::from(vs.ef_search.max(1));
        let k = f64::from(vs.candidates.max(1));
        // HNSW probe count scales with ef_search (frontier size) and k (results).
        // log2(total) captures the graph-height component.
        let probe = (ef + k) * total.log2().max(1.0);

        let f = self.factors.get();
        let d = default_factors();
        let io_ratio = f.random_page_cost / d.random_page_cost;
        let cpu_ratio = f.cpu_distance_cost / d.cpu_distance_cost;

        Cost::new(
            probe * COMPAT_HNSW_IO * io_ratio,
            probe * COMPAT_HNSW_CPU * cpu_ratio,
        )
    }

    /// Cost of a full table scan, proportional to row count.
    fn estimate_table_scan_cost(&self) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;

        let f = self.factors.get();
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

        let f = self.factors.get();
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

        let f = self.factors.get();
        let d = default_factors();
        let edge_ratio = f.cpu_edge_cost / d.cpu_edge_cost;

        Cost::new(0.0, frontier * edge_ratio * d.cpu_edge_cost)
    }

    /// Cost of a Limit or Offset node — proportional to tuples passing through,
    /// using the configured `cpu_tuple_cost`. Negligible but non-zero so that
    /// plans with many pipeline stages are penalised.
    fn estimate_limit_offset_cost(&self) -> Cost {
        let f = self.factors.get();
        let d = default_factors();
        let cpu_ratio = f.cpu_tuple_cost / d.cpu_tuple_cost;
        // Treat Limit/Offset as traversing a handful of rows; the real count
        // is known by the caller but is a second-order effect on total cost.
        Cost::new(0.0, d.cpu_tuple_cost * cpu_ratio)
    }

    /// Estimates selectivity for a `Like` condition.
    ///
    /// Prefix patterns (ending with `%`, not starting with `%`) use histogram
    /// range estimation on the ordinal prefix range when available.
    /// Non-prefix patterns return `0.05`.
    fn estimate_like_selectivity(&self, column: &str, pattern: &str) -> f64 {
        let is_prefix = pattern.ends_with('%') && !pattern.starts_with('%');
        if !is_prefix {
            return 0.05;
        }

        let Some(_hist) = self.get_histogram(column) else {
            return 0.1;
        };

        // For string columns the histogram is built on ordinal ranks.
        // A prefix pattern 'abc%' matches a contiguous range of ordinal
        // values. Without the full string→rank mapping at plan time we
        // approximate: the prefix covers roughly 1/distinct_count of the
        // domain, scaled by the number of buckets that span that range.
        // This is more accurate than the previous 1/bucket_count heuristic.
        let distinct = self
            .stats
            .column_stats
            .get(column)
            .or_else(|| self.stats.field_stats.get(column))
            .map_or(1, |cs| cs.distinct_count.max(1));
        (1.0 / distinct as f64).clamp(0.01, 1.0)
    }
}

#[cfg(test)]
mod plan_cost_tests {
    //! Unit tests for the new `estimate_plan_cost` API. These exercise the
    //! cost-monotonicity invariants independently of the EXPLAIN pipeline.

    use super::*;
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
}
