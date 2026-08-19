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
fn method_isnull_heuristic_when_collection_is_empty() {
    // Devin finding J on PR #606: `is_null_selectivity_with_method` must
    // apply the same `total_points > 0` guard as `has_cardinality_data`.
    // When the collection is empty, `null_count / 1` does not reflect
    // real data and the method must be Heuristic, not Cardinality.
    let mut stats = CollectionStats::new();
    stats.total_points = 0;
    stats.row_count = 0;
    let mut cs = ColumnStats::new("optional_field");
    cs.null_count = 5; // stale value on an empty collection
    stats.field_stats.insert("optional_field".into(), cs);
    let est = CostEstimator::new(&stats);
    let cond = Condition::IsNull(crate::velesql::ast::IsNullCondition {
        column: "optional_field".into(),
        is_null: true,
    });
    let (_sel, method) = est.estimate_condition_selectivity_with_method(&cond);
    assert_eq!(
        method,
        SelectivityMethod::Heuristic,
        "IsNull on empty collection must classify as Heuristic"
    );
}

#[test]
fn method_isnull_cardinality_when_column_tracked() {
    // Companion to `method_isnull_heuristic_when_collection_is_empty`:
    // a populated collection with a tracked column must still classify
    // IsNull as Cardinality (guard-rail regression check).
    let mut stats = CollectionStats::new();
    stats.total_points = 1_000;
    stats.row_count = 1_000;
    let mut cs = ColumnStats::new("optional_field");
    cs.null_count = 12;
    stats.field_stats.insert("optional_field".into(), cs);
    let est = CostEstimator::new(&stats);
    let cond = Condition::IsNull(crate::velesql::ast::IsNullCondition {
        column: "optional_field".into(),
        is_null: true,
    });
    let (_sel, method) = est.estimate_condition_selectivity_with_method(&cond);
    assert_eq!(method, SelectivityMethod::Cardinality);
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
