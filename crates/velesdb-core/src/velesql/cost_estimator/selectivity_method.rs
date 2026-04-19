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
}

#[cfg(test)]
mod tests {
    //! Tests for [`SelectivityMethod`] propagation (issue #471, Devin finding 2).
    //!
    //! Verifies that `estimate_condition_selectivity_with_method` returns the
    //! actual method used (histogram / cardinality / heuristic), and that
    //! compound predicates report the worst-case method among their children.

    use super::*;
    use crate::collection::stats::{CollectionStats, ColumnStats, Histogram, HistogramBucket};
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
