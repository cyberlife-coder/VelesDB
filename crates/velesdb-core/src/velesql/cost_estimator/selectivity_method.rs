//! Method-aware selectivity estimation (issue #471, Devin finding 2).
//!
//! Extracted from the monolithic `cost_estimator.rs` to respect the 500 NLOC
//! file limit (Devin Finding F on PR #606). Provides variants of the base
//! selectivity helpers that also return the [`SelectivityMethod`] used to
//! compute the estimate (histogram / cardinality / heuristic), so EXPLAIN
//! can report the confidence level of each predicate.

use super::{value_to_f64, CostEstimator, SelectivityMethod};
use crate::velesql::ast::{CompareOp, Condition, Value};

impl CostEstimator<'_> {
    /// Same as [`CostEstimator::estimate_condition_selectivity`] but also
    /// returns the [`SelectivityMethod`] that produced the estimate (issue
    /// #471, Devin finding 2). For compound predicates, returns the
    /// worst-case method among children so EXPLAIN never overstates
    /// confidence.
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
    /// actually be used by [`crate::collection::stats::CollectionStats::estimate_selectivity`]
    /// — i.e. when the selectivity estimate would NOT fall back to the
    /// hard-coded `0.1` heuristic.
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

    /// Method-aware variant of [`CostEstimator::estimate_comparison_selectivity_with_histogram`].
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

    /// Method-aware variant of [`CostEstimator::estimate_in_selectivity`].
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

    /// Method-aware variant of [`CostEstimator::estimate_between_selectivity`].
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

    /// Method-aware variant of [`CostEstimator::estimate_like_selectivity`].
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
    ///
    /// Mirrors the `total_points > 0` guard used by `has_cardinality_data`:
    /// when the collection is empty (or stats are corrupted with
    /// `total_points == 0`), the computed `null_count / 1` selectivity does
    /// not reflect real data, so the method must be `Heuristic` rather than
    /// `Cardinality` — Devin finding J on PR #606.
    fn is_null_selectivity_with_method(&self, column: &str) -> (f64, SelectivityMethod) {
        let sel = self.stats.field_stats.get(column).map_or(0.1, |s| {
            s.null_count as f64 / self.stats.total_points.max(1) as f64
        });
        let method = if self.stats.field_stats.contains_key(column) && self.stats.total_points > 0 {
            SelectivityMethod::Cardinality
        } else {
            SelectivityMethod::Heuristic
        };
        (sel, method)
    }
}

#[cfg(test)]
#[path = "selectivity_method_tests.rs"]
mod tests;
