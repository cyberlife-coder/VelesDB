//! Cost estimator for hybrid MATCH + NEAR query planning.

// Reason: usize/u64 → f64 for selectivity ratios and log2 inputs; these are
// cardinalities where ±1 ULP has no operational impact on query planning.
#![allow(clippy::cast_precision_loss)]

use crate::collection::stats::{CollectionStats, Histogram};
use crate::velesql::ast::Condition;

const FILTER_SCAN_IO_WEIGHT: f64 = 0.2;
const FILTER_SCAN_CPU_WEIGHT: f64 = 0.8;
const HNSW_IO_WEIGHT: f64 = 0.5;
const HNSW_CPU_WEIGHT: f64 = 1.0;

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

/// Cost estimator based on collection statistics.
#[derive(Debug)]
pub struct CostEstimator<'a> {
    stats: &'a CollectionStats,
}

impl<'a> CostEstimator<'a> {
    #[must_use]
    /// Creates a new estimator backed by collection statistics.
    pub const fn new(stats: &'a CollectionStats) -> Self {
        Self { stats }
    }

    #[must_use]
    /// Estimates filter cost using selectivity derived from stats.
    pub fn estimate_filter_cost(&self, filter: &Condition) -> Cost {
        let selectivity = self.estimate_condition_selectivity(filter).clamp(0.0, 1.0);
        let total = self.stats.total_points.max(self.stats.row_count) as f64;
        let scan_rows = (total * selectivity).max(1.0);
        Cost::new(
            scan_rows * FILTER_SCAN_IO_WEIGHT,
            scan_rows * FILTER_SCAN_CPU_WEIGHT,
        )
    }

    #[must_use]
    /// Estimates HNSW search cost for top-k retrieval.
    pub fn estimate_hnsw_search_cost(&self, k: usize) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1) as f64;
        let probe = (k.max(1) as f64) * total.log2().max(1.0);
        Cost::new(probe * HNSW_IO_WEIGHT, probe * HNSW_CPU_WEIGHT)
    }

    #[must_use]
    /// Estimates predicate selectivity in the [0.0, 1.0] range.
    pub fn estimate_condition_selectivity(&self, condition: &Condition) -> f64 {
        match condition {
            Condition::Comparison(cmp) => self.estimate_comparison_selectivity(cmp.column.as_str()),
            Condition::In(cond) => {
                let base = self.stats.estimate_selectivity(cond.column.as_str());
                (base * cond.values.len() as f64).clamp(0.0, 1.0)
            }
            Condition::Between(cond) => self.estimate_range_selectivity(cond.column.as_str()),
            Condition::Like(cond) => {
                (self.stats.estimate_selectivity(cond.column.as_str()) * 2.0).clamp(0.01, 1.0)
            }
            Condition::IsNull(cond) => self
                .stats
                .field_stats
                .get(cond.column.as_str())
                .map_or(0.1, |s| {
                    s.null_count as f64 / self.stats.total_points.max(1) as f64
                }),
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
            _ => 0.5,
        }
    }

    fn estimate_comparison_selectivity(&self, column: &str) -> f64 {
        self.stats.estimate_selectivity(column).clamp(0.001, 1.0)
    }

    fn estimate_range_selectivity(&self, column: &str) -> f64 {
        self.stats
            .field_stats
            .get(column)
            .and_then(|s| s.histogram.as_ref())
            .map_or(0.3, histogram_range_selectivity)
    }
}

fn histogram_range_selectivity(histogram: &Histogram) -> f64 {
    if histogram.buckets.is_empty() {
        return 0.3;
    }
    1.0 / histogram.buckets.len() as f64
}
