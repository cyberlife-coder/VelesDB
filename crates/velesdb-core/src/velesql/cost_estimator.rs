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

/// Source of a selectivity estimate, used by EXPLAIN to report how a
/// predicate's selectivity was computed (issue #471, Devin finding 2).
///
/// Ordered by increasing noise / decreasing confidence:
/// 1. `Histogram` — derived from calibrated histogram buckets (most accurate).
/// 2. `Cardinality` — derived from `distinct_count` only (no distribution).
/// 3. `Heuristic` — hard-coded constant (e.g. 0.1 for `Match`, 0.05 for
///    `ContainsText`) because the predicate type has no stats path at all.
///
/// For compound predicates (`And`/`Or`/`Not`/`Group`), the reported method is
/// the **worst case** among children in the order above, so EXPLAIN never
/// overstates its confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectivityMethod {
    /// Selectivity computed from histogram bucket data.
    Histogram,
    /// Selectivity computed from `distinct_count` cardinality (no histogram).
    Cardinality,
    /// Selectivity computed from a heuristic constant.
    Heuristic,
}

impl SelectivityMethod {
    /// Returns the EXPLAIN display label for this method.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Histogram => "histogram",
            Self::Cardinality => "cardinality",
            Self::Heuristic => "heuristic",
        }
    }

    /// Returns the worst (least confident) of two methods.
    ///
    /// Ordering: `Heuristic > Cardinality > Histogram`. Used to combine the
    /// methods of sub-predicates under `And`/`Or`/`Not`/`Group` so the
    /// reported method reflects the loosest child.
    #[must_use]
    pub const fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::Heuristic, _) | (_, Self::Heuristic) => Self::Heuristic,
            (Self::Cardinality, _) | (_, Self::Cardinality) => Self::Cardinality,
            _ => Self::Histogram,
        }
    }
}

impl std::fmt::Display for SelectivityMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
        self.hnsw_cost_from_probe(probe)
    }

    /// Estimates HNSW search cost parametrized by the actual `ef_search`
    /// (frontier size) and `candidates` (top-k request) — issue #471, Devin
    /// finding 4.
    ///
    /// Uses the same `(ef + k) * log2(total)` probe formula as
    /// [`Self::estimate_vector_search_node_cost`], so callers that have
    /// `ef_search` / `candidates` available (e.g. pre/post-filter strategy
    /// comparison in `plan_builder`) get a cost that reflects the real query
    /// instead of a fixed `k = 10`.
    #[must_use]
    pub fn estimate_hnsw_search_cost_with_ef(&self, ef_search: u32, candidates: u32) -> Cost {
        let total = self.stats.total_points.max(self.stats.row_count).max(1);
        self.estimate_hnsw_search_cost_with_ef_on_size(ef_search, candidates, total)
    }

    /// Variant of [`Self::estimate_hnsw_search_cost_with_ef`] that takes the
    /// effective collection size explicitly.
    ///
    /// Callers use this when the HNSW pass runs over a **subset** of the
    /// collection — typically the surviving rows after a pre-filter. Modeling
    /// the cost as `(ef + k) * log2(collection_size)` preserves the
    /// logarithmic scaling HNSW actually exhibits, whereas multiplying the
    /// full-collection cost by the filter selectivity would imply linear
    /// scaling in the reduced size (Devin finding E on PR #606).
    #[must_use]
    pub fn estimate_hnsw_search_cost_with_ef_on_size(
        &self,
        ef_search: u32,
        candidates: u32,
        collection_size: u64,
    ) -> Cost {
        let total = collection_size.max(1) as f64;
        let ef = f64::from(ef_search.max(1));
        let k = f64::from(candidates.max(1));
        let probe = (ef + k) * total.log2().max(1.0);
        self.hnsw_cost_from_probe(probe)
    }

    /// Applies calibrated I/O and CPU weights to a raw HNSW probe count.
    ///
    /// Single source of truth for the `Cost::new(probe * io_w, probe * cpu_w)`
    /// formula used by every HNSW cost helper — avoids duplicating the
    /// factor-ratio resolution in three places.
    fn hnsw_cost_from_probe(&self, probe: f64) -> Cost {
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

    /// Same as [`Self::estimate_condition_selectivity`] but also returns the
    /// [`SelectivityMethod`] that produced the estimate (issue #471, Devin
    /// finding 2). For compound predicates, returns the worst-case method
    /// among children so EXPLAIN never overstates confidence.
    #[must_use]
    pub fn estimate_condition_selectivity_with_method(
        &self,
        condition: &Condition,
    ) -> (f64, SelectivityMethod) {
        match condition {
            Condition::Comparison(cmp) => {
                self.comparison_selectivity_with_method(&cmp.column, cmp.operator, &cmp.value)
            }
            Condition::In(cond) => {
                self.in_selectivity_with_method(&cond.column, &cond.values, cond.negated)
            }
            Condition::Between(cond) => {
                self.between_selectivity_with_method(&cond.column, &cond.low, &cond.high)
            }
            Condition::Like(cond) => self.like_selectivity_with_method(&cond.column, &cond.pattern),
            Condition::IsNull(cond) => self.is_null_selectivity_with_method(&cond.column),
            Condition::Match(_)
            | Condition::Contains(_)
            | Condition::GeoDistance(_)
            | Condition::ContainsText(_)
            | Condition::GeoBbox(_)
            | Condition::GraphMatch(_) => (
                self.estimate_condition_selectivity(condition),
                SelectivityMethod::Heuristic,
            ),
            Condition::And(left, right) => {
                let (l, ml) = self.estimate_condition_selectivity_with_method(left);
                let (r, mr) = self.estimate_condition_selectivity_with_method(right);
                (l * r, ml.worst(mr))
            }
            Condition::Or(left, right) => {
                let (l, ml) = self.estimate_condition_selectivity_with_method(left);
                let (r, mr) = self.estimate_condition_selectivity_with_method(right);
                ((l + r - (l * r)).clamp(0.0, 1.0), ml.worst(mr))
            }
            Condition::Not(inner) => {
                let (s, m) = self.estimate_condition_selectivity_with_method(inner);
                (1.0 - s, m)
            }
            Condition::Group(inner) => self.estimate_condition_selectivity_with_method(inner),
            Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_)
            | Condition::SparseVectorSearch(_)
            | Condition::Similarity(_) => (1.0, SelectivityMethod::Heuristic),
        }
    }

    /// Returns `true` when `column` has usable cardinality data that would
    /// actually be used by [`CollectionStats::estimate_selectivity`] — i.e.
    /// when the selectivity estimate would NOT fall back to the hard-coded
    /// `0.1` heuristic.
    ///
    /// Mirrors the exact preconditions of `estimate_selectivity`
    /// (`collection/stats/mod.rs`): the column must have a non-zero
    /// distinct count AND the collection must have a non-zero total
    /// (`total_points` for `field_stats`, `row_count` for `column_stats`).
    /// Without the total check, an empty or corrupted collection with
    /// `total_points == 0` but `distinct_values > 0` would be misclassified
    /// as `SelectivityMethod::Cardinality` even though the underlying
    /// estimator returned the heuristic 0.1 (Devin finding H on PR #606).
    fn has_cardinality_data(&self, column: &str) -> bool {
        let field_has = self
            .stats
            .field_stats
            .get(column)
            .is_some_and(|s| s.distinct_values > 0)
            && self.stats.total_points > 0;
        let column_has = self
            .stats
            .column_stats
            .get(column)
            .is_some_and(|s| s.distinct_count > 0)
            && self.stats.row_count > 0;
        field_has || column_has
    }

    /// Method-aware variant of [`Self::estimate_comparison_selectivity_with_histogram`].
    fn comparison_selectivity_with_method(
        &self,
        column: &str,
        op: CompareOp,
        value: &Value,
    ) -> (f64, SelectivityMethod) {
        let sel = self.estimate_comparison_selectivity_with_histogram(column, op, value);
        let method = if matches!(value, Value::Parameter(_)) {
            SelectivityMethod::Heuristic
        } else if value_to_f64(value).is_some() && self.get_histogram(column).is_some() {
            SelectivityMethod::Histogram
        } else if self.has_cardinality_data(column) {
            SelectivityMethod::Cardinality
        } else {
            SelectivityMethod::Heuristic
        };
        (sel, method)
    }

    /// Method-aware variant of [`Self::estimate_in_selectivity`].
    fn in_selectivity_with_method(
        &self,
        column: &str,
        values: &[Value],
        negated: bool,
    ) -> (f64, SelectivityMethod) {
        let sel = self.estimate_in_selectivity(column, values, negated);
        let has_numeric = values.iter().any(|v| value_to_f64(v).is_some());
        let method = if has_numeric && self.get_histogram(column).is_some() {
            SelectivityMethod::Histogram
        } else if self.has_cardinality_data(column) {
            SelectivityMethod::Cardinality
        } else {
            SelectivityMethod::Heuristic
        };
        (sel, method)
    }

    /// Method-aware variant of [`Self::estimate_between_selectivity`].
    fn between_selectivity_with_method(
        &self,
        column: &str,
        low: &Value,
        high: &Value,
    ) -> (f64, SelectivityMethod) {
        let sel = self.estimate_between_selectivity(column, low, high);
        let numeric = value_to_f64(low).is_some() && value_to_f64(high).is_some();
        let method = if numeric && self.get_histogram(column).is_some() {
            SelectivityMethod::Histogram
        } else {
            SelectivityMethod::Heuristic
        };
        (sel, method)
    }

    /// Method-aware variant of [`Self::estimate_like_selectivity`].
    fn like_selectivity_with_method(
        &self,
        column: &str,
        pattern: &str,
    ) -> (f64, SelectivityMethod) {
        let sel = self.estimate_like_selectivity(column, pattern);
        let is_prefix = pattern.ends_with('%') && !pattern.starts_with('%');
        let method = if is_prefix && self.get_histogram(column).is_some() {
            SelectivityMethod::Cardinality
        } else {
            SelectivityMethod::Heuristic
        };
        (sel, method)
    }

    /// Method-aware variant for `IsNull`.
    fn is_null_selectivity_with_method(&self, column: &str) -> (f64, SelectivityMethod) {
        let sel = self.stats.field_stats.get(column).map_or(0.1, |s| {
            s.null_count as f64 / self.stats.total_points.max(1) as f64
        });
        let method = if self.stats.field_stats.contains_key(column) {
            SelectivityMethod::Cardinality
        } else {
            SelectivityMethod::Heuristic
        };
        (sel, method)
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
    ///
    /// Uses the same `(ef + k) * log2(total)` probe formula as the public
    /// HNSW cost helpers and delegates the probe → Cost conversion to
    /// [`Self::hnsw_cost_from_probe`], so a future change to the HNSW
    /// factor-ratio model updates all three call sites at once.
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

#[cfg(test)]
mod selectivity_method_tests {
    //! Tests for [`SelectivityMethod`] propagation (issue #471, Devin finding 2).
    //!
    //! Verifies that `estimate_condition_selectivity_with_method` returns the
    //! actual method used (histogram / cardinality / heuristic), and that
    //! compound predicates report the worst-case method among their children.

    use super::*;
    use crate::collection::stats::{ColumnStats, Histogram, HistogramBucket};
    use crate::velesql::ast::{Comparison, Condition, MatchCondition, Value};

    /// Builds a `CollectionStats` with `total` rows and an optional histogram
    /// on column `col`.
    fn stats_with_col(total: u64, col: &str, with_hist: bool) -> CollectionStats {
        let mut s = CollectionStats::new();
        s.total_points = total;
        s.row_count = total;
        let mut cs = ColumnStats::new(col).with_distinct_count(100);
        if with_hist {
            cs.histogram = Some(Histogram {
                buckets: vec![HistogramBucket {
                    lower_bound: 0.0,
                    upper_bound: 1000.0,
                    count: total,
                    distinct_count: 100,
                }],
                total_count: total,
                incremental_updates: 0,
                stale: false,
            });
        }
        s.column_stats.insert(col.to_string(), cs.clone());
        s.field_stats.insert(col.to_string(), cs);
        s
    }

    fn cmp_eq(col: &str, v: i64) -> Condition {
        Condition::Comparison(Comparison {
            column: col.to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(v),
        })
    }

    fn cmp_param(col: &str) -> Condition {
        Condition::Comparison(Comparison {
            column: col.to_string(),
            operator: CompareOp::Eq,
            value: Value::Parameter("v".into()),
        })
    }

    #[test]
    fn method_histogram_when_numeric_value_and_histogram_present() {
        let stats = stats_with_col(1_000, "price", true);
        let est = CostEstimator::new(&stats);
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&cmp_eq("price", 42));
        assert_eq!(method, SelectivityMethod::Histogram);
    }

    #[test]
    fn method_cardinality_when_no_histogram() {
        let stats = stats_with_col(1_000, "price", false);
        let est = CostEstimator::new(&stats);
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&cmp_eq("price", 42));
        assert_eq!(method, SelectivityMethod::Cardinality);
    }

    #[test]
    fn method_heuristic_when_column_unknown() {
        // `price` has no entry in field_stats nor column_stats — the underlying
        // CollectionStats::estimate_selectivity falls back to the 0.1 heuristic.
        // The method must be Heuristic, not Cardinality (Devin finding B, #606).
        let mut stats = CollectionStats::new();
        stats.total_points = 1_000;
        stats.row_count = 1_000;
        let est = CostEstimator::new(&stats);
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&cmp_eq("price", 42));
        assert_eq!(
            method,
            SelectivityMethod::Heuristic,
            "Unknown columns must report Heuristic, not Cardinality"
        );
    }

    #[test]
    fn method_heuristic_when_cardinality_data_is_empty() {
        // Column exists in field_stats but with distinct_values == 0
        // (e.g. stats object initialised but never populated).
        let mut stats = CollectionStats::new();
        stats.total_points = 1_000;
        stats.row_count = 1_000;
        let empty = ColumnStats::new("price"); // distinct_values defaults to 0
        stats.field_stats.insert("price".into(), empty);
        let est = CostEstimator::new(&stats);
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&cmp_eq("price", 42));
        assert_eq!(method, SelectivityMethod::Heuristic);
    }

    #[test]
    fn method_in_heuristic_when_column_unknown() {
        // IN predicate on unknown column must also classify as Heuristic.
        let mut stats = CollectionStats::new();
        stats.total_points = 1_000;
        stats.row_count = 1_000;
        let est = CostEstimator::new(&stats);
        let cond = Condition::In(crate::velesql::ast::InCondition {
            column: "tag".into(),
            values: vec![Value::String("a".into()), Value::String("b".into())],
            negated: false,
        });
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&cond);
        assert_eq!(method, SelectivityMethod::Heuristic);
    }

    #[test]
    fn method_heuristic_when_column_has_distinct_but_collection_is_empty() {
        // Edge case (Devin finding H on #606): the column has distinct data
        // in field_stats but the collection itself has total_points == 0
        // (e.g. corrupted or manually-constructed stats). The underlying
        // `CollectionStats::estimate_selectivity` falls back to 0.1 in
        // this case, so `has_cardinality_data` must return false and the
        // method must be `Heuristic`, not `Cardinality`.
        let mut stats = CollectionStats::new();
        stats.total_points = 0; // empty / corrupted
        stats.row_count = 0;
        let stale = ColumnStats::new("price").with_distinct_count(100);
        stats.field_stats.insert("price".into(), stale.clone());
        stats.column_stats.insert("price".into(), stale);
        let est = CostEstimator::new(&stats);
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&cmp_eq("price", 42));
        assert_eq!(
            method,
            SelectivityMethod::Heuristic,
            "empty collection with stale cardinality must degrade to Heuristic"
        );
    }

    #[test]
    fn method_heuristic_when_parameter_value() {
        let stats = stats_with_col(1_000, "price", true);
        let est = CostEstimator::new(&stats);
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&cmp_param("price"));
        assert_eq!(
            method,
            SelectivityMethod::Heuristic,
            "Parameter values are unknown at plan time → Heuristic"
        );
    }

    #[test]
    fn method_heuristic_for_match_predicate() {
        let stats = stats_with_col(1_000, "body", true);
        let est = CostEstimator::new(&stats);
        let cond = Condition::Match(MatchCondition {
            column: "body".into(),
            query: "hello".into(),
        });
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&cond);
        assert_eq!(method, SelectivityMethod::Heuristic);
    }

    #[test]
    fn method_compound_and_takes_worst_case() {
        // AND(histogram_cond, heuristic_cond) → Heuristic (worst case).
        let stats = stats_with_col(1_000, "price", true);
        let est = CostEstimator::new(&stats);

        let histogram_cond = cmp_eq("price", 42);
        let heuristic_cond = Condition::Match(MatchCondition {
            column: "body".into(),
            query: "x".into(),
        });
        let compound = Condition::And(Box::new(histogram_cond), Box::new(heuristic_cond));

        let (_sel, method) = est.estimate_condition_selectivity_with_method(&compound);
        assert_eq!(
            method,
            SelectivityMethod::Heuristic,
            "AND of (Histogram, Heuristic) must report Heuristic (worst case)"
        );
    }

    #[test]
    fn method_compound_or_takes_worst_case() {
        let stats = stats_with_col(1_000, "price", true);
        let est = CostEstimator::new(&stats);

        let histogram_cond = cmp_eq("price", 42);
        let cardinality_cond = cmp_param("price"); // Parameter → heuristic, actually

        // To get pure cardinality: drop histogram, keep column_stats.
        let stats_card = stats_with_col(1_000, "other", false);
        let est_card = CostEstimator::new(&stats_card);
        let card_cond = cmp_eq("other", 10);

        // Assert cardinality path is detected on its own.
        let (_, m1) = est_card.estimate_condition_selectivity_with_method(&card_cond);
        assert_eq!(m1, SelectivityMethod::Cardinality);

        // Now verify OR(histogram, heuristic) = heuristic.
        let compound = Condition::Or(Box::new(histogram_cond), Box::new(cardinality_cond));
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&compound);
        assert_eq!(method, SelectivityMethod::Heuristic);
    }

    #[test]
    fn method_not_preserves_child_method() {
        let stats = stats_with_col(1_000, "price", true);
        let est = CostEstimator::new(&stats);
        let inner = cmp_eq("price", 42);
        let not_cond = Condition::Not(Box::new(inner));
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&not_cond);
        assert_eq!(method, SelectivityMethod::Histogram);
    }

    #[test]
    fn method_group_preserves_child_method() {
        let stats = stats_with_col(1_000, "price", true);
        let est = CostEstimator::new(&stats);
        let inner = cmp_eq("price", 42);
        let grouped = Condition::Group(Box::new(inner));
        let (_sel, method) = est.estimate_condition_selectivity_with_method(&grouped);
        assert_eq!(method, SelectivityMethod::Histogram);
    }

    #[test]
    fn method_str_labels_match_explain_display() {
        assert_eq!(SelectivityMethod::Histogram.as_str(), "histogram");
        assert_eq!(SelectivityMethod::Cardinality.as_str(), "cardinality");
        assert_eq!(SelectivityMethod::Heuristic.as_str(), "heuristic");
    }

    #[test]
    fn backward_compat_selectivity_value_unchanged() {
        // The non-method-aware function must return the same selectivity as
        // the method-aware one; refactor must not alter numeric outputs.
        let stats = stats_with_col(1_000, "price", true);
        let est = CostEstimator::new(&stats);
        let cond = cmp_eq("price", 42);

        let sel_new = est.estimate_condition_selectivity_with_method(&cond).0;
        let sel_old = est.estimate_condition_selectivity(&cond);
        assert!(
            (sel_new - sel_old).abs() < f64::EPSILON,
            "method-aware and legacy paths must agree: new={sel_new} old={sel_old}"
        );
    }
}
