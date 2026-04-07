//! Cost estimator for hybrid MATCH + NEAR query planning.

// Reason: usize/u64 → f64 for selectivity ratios and log2 inputs; these are
// cardinalities where ±1 ULP has no operational impact on query planning.
#![allow(clippy::cast_precision_loss)]

use crate::collection::stats::next_after;
use crate::collection::stats::CollectionStats;
use crate::collection::stats::Histogram;
use crate::velesql::ast::{CompareOp, Condition, Value};

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

impl<'a> CostEstimator<'a> {
    #[must_use]
    /// Creates a new estimator backed by collection statistics.
    pub const fn new(stats: &'a CollectionStats) -> Self {
        Self { stats }
    }

    /// Returns the histogram for a column, delegating to `CollectionStats`.
    fn get_histogram(&self, column: &str) -> Option<&Histogram> {
        self.stats.get_column_histogram(column)
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

        let v = match value_to_f64(value) {
            Some(v) => v,
            None => return self.stats.estimate_selectivity(column),
        };

        let hist = match self.get_histogram(column) {
            Some(h) => h,
            None => return self.stats.estimate_selectivity(column),
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
        let (low_f, high_f) = match (value_to_f64(low), value_to_f64(high)) {
            (Some(l), Some(h)) => (l, h),
            _ => return 0.3,
        };

        match self.get_histogram(column) {
            Some(h) => h.estimate_range_selectivity(low_f, high_f),
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

    /// Estimates selectivity for a `Like` condition.
    ///
    /// Prefix patterns (ending with `%`, not starting with `%`) use histogram
    /// range estimation when available, otherwise return `0.1`.
    /// Non-prefix patterns return `0.05`.
    fn estimate_like_selectivity(&self, column: &str, pattern: &str) -> f64 {
        let is_prefix = pattern.ends_with('%') && !pattern.starts_with('%');
        if !is_prefix {
            return 0.05;
        }

        match self.get_histogram(column) {
            Some(h) if h.total_count > 0 => {
                // Use a fraction of the histogram range as a rough estimate.
                let bucket_count = h.buckets.len() as f64;
                (1.0 / bucket_count).clamp(0.01, 1.0)
            }
            _ => 0.1,
        }
    }
}
