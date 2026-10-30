//! Tests for [`super::grouped_order`]: which column of a grouped row an
//! ORDER BY item sorts on, and the `V016` refusal of one that names none.

use super::ast::{AggregateArg, AggregateFunction, OrderByExpr, SelectColumns, SelectStatement};
use super::validation::{QueryValidator, ValidationError, ValidationErrorKind};
use super::{vector_group_score_is_average, GroupedSortKey, Parser};

fn validate(sql: &str) -> Result<(), ValidationError> {
    QueryValidator::validate(&Parser::parse(sql).expect("test: parse"))
}

fn assert_v016(sql: &str) {
    let err = validate(sql).expect_err("test: ORDER BY must be refused");
    assert_eq!(err.kind, ValidationErrorKind::OrderByNotInResult, "{err}");
    assert_eq!(err.kind.code(), "V016");
}

#[test]
fn test_validator_refuses_a_grouped_order_by_aggregate_missing_from_select() {
    assert_v016("SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY SUM(year)");
}

#[test]
fn test_validator_refuses_a_grouped_order_by_column_outside_group_by() {
    assert_v016("SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY year");
}

#[test]
fn test_validator_refuses_similarity_on_plain_grouped_rows() {
    // A similarity() threshold gives a score context but no vector GROUP BY:
    // the grouped rows carry no score to sort on.
    assert_v016(
        "SELECT category, COUNT(*) AS n FROM docs WHERE similarity(vector, $v) > 0.5 \
         GROUP BY category ORDER BY similarity() DESC",
    );
}

#[test]
fn test_validator_refuses_arithmetic_on_plain_grouped_rows() {
    assert_v016("SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY n * 2");
}

#[test]
fn test_validator_accepts_alias_group_column_and_default_name() {
    validate("SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY n, category")
        .expect("test: alias and group column");
    validate("SELECT category, COUNT(*) FROM docs GROUP BY category ORDER BY count")
        .expect("test: core default name");
}

#[test]
fn test_validator_refuses_a_vector_group_by_order_by_aggregate_missing_from_select() {
    // Refused before execution, so EXPLAIN and every executor see it too.
    assert_v016(
        "SELECT category, MAX(score) AS best FROM docs WHERE vector NEAR $v \
         GROUP BY category ORDER BY AVG(score)",
    );
}

#[test]
fn test_validator_leaves_similarity_and_arithmetic_to_a_vector_group_by() {
    validate(
        "SELECT category, MAX(score) AS best FROM docs WHERE vector NEAR $v \
         GROUP BY category ORDER BY similarity() DESC, MAX(score) DESC",
    )
    .expect("test: vector GROUP BY sorts on score");
}

// ---------------------------------------------------------------------------
// grouped_sort_keys: name precedence and the executor's default names
// ---------------------------------------------------------------------------

fn select(sql: &str) -> SelectStatement {
    Parser::parse(sql).expect("test: parse").select
}

/// WASM-style default names: `count(*)`, `sum(year)`.
fn call_shaped(agg: &AggregateFunction) -> String {
    let function = format!("{:?}", agg.function_type).to_lowercase();
    match &agg.argument {
        AggregateArg::Column(column) => format!("{function}({column})"),
        _ => format!("{function}(*)"),
    }
}

#[test]
fn test_an_explicit_alias_outranks_a_group_column_of_the_same_name() {
    let stmt = select(
        "SELECT category, COUNT(*) AS category FROM docs GROUP BY category ORDER BY category",
    );
    let keys = stmt
        .grouped_sort_keys(AggregateFunction::result_key)
        .expect("test: resolve");
    assert!(
        matches!(keys[0].0, GroupedSortKey::Aggregate(_)),
        "{keys:?}"
    );
}

#[test]
fn test_a_group_column_outranks_a_default_aggregate_name() {
    let stmt = select("SELECT count, COUNT(*) FROM docs GROUP BY count ORDER BY count DESC");
    let keys = stmt
        .grouped_sort_keys(AggregateFunction::result_key)
        .expect("test: resolve");
    assert_eq!(keys, vec![(GroupedSortKey::GroupColumn("count"), true)]);
}

#[test]
fn test_a_default_name_resolves_with_the_executors_own_naming() {
    let stmt = select("SELECT category, COUNT(*) FROM docs GROUP BY category ORDER BY count");
    // Core outputs COUNT(*) as `count`...
    let keys = stmt
        .grouped_sort_keys(AggregateFunction::result_key)
        .expect("test: core");
    assert!(
        matches!(keys[0].0, GroupedSortKey::Aggregate(_)),
        "{keys:?}"
    );
    // ...WASM as `count(*)`, so `count` names no column of its rows.
    let err = stmt.grouped_sort_keys(call_shaped).expect_err("test: wasm");
    assert_eq!(err.kind.code(), "V016");
}

#[test]
fn test_a_default_name_never_matches_an_aliased_aggregate() {
    let stmt = select("SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY count");
    assert!(stmt
        .grouped_sort_keys(AggregateFunction::result_key)
        .is_err());
}

#[test]
fn test_a_repeated_aggregate_resolves_to_its_select_item_whatever_the_alias() {
    let stmt = select(
        "SELECT category, AVG(year) AS mean FROM docs GROUP BY category ORDER BY AVG(year) DESC",
    );
    let keys = stmt
        .grouped_sort_keys(AggregateFunction::result_key)
        .expect("test: resolve");
    let GroupedSortKey::Aggregate(agg) = keys[0].0 else {
        panic!("expected an aggregate key, got {keys:?}");
    };
    assert_eq!(agg.alias.as_deref(), Some("mean"));
    assert!(keys[0].1, "DESC is kept");
}

// ---------------------------------------------------------------------------
// order_by_aggregates_as_fields and the vector GROUP BY score
// ---------------------------------------------------------------------------

#[test]
fn test_order_by_aggregates_become_the_executors_field_and_the_rest_is_kept() {
    let stmt = select(
        "SELECT category, MAX(score) AS best FROM docs WHERE vector NEAR $v \
         GROUP BY category ORDER BY MAX(score) DESC, similarity() ASC",
    );
    let rewritten = stmt
        .order_by_aggregates_as_fields(|agg| format!("field-of-{}", agg.result_key()))
        .expect("test: rewrite");
    let order_by = rewritten.order_by.as_deref().expect("test: order by");
    assert_eq!(
        order_by[0].expr,
        OrderByExpr::Field("field-of-best".to_string())
    );
    assert!(order_by[0].descending);
    assert_eq!(order_by[1].expr, OrderByExpr::SimilarityBare);
}

#[test]
fn test_order_by_without_aggregates_is_borrowed_unchanged() {
    let stmt = select(
        "SELECT category, MAX(score) AS best FROM docs WHERE vector NEAR $v \
         GROUP BY category ORDER BY similarity() DESC",
    );
    let same = stmt
        .order_by_aggregates_as_fields(AggregateFunction::result_key)
        .expect("test: no rewrite");
    assert!(matches!(same, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn test_a_vector_group_scores_by_average_only_when_avg_score_is_selected_without_max() {
    let aggregations = |sql: &str| match select(sql).columns {
        SelectColumns::Mixed { aggregations, .. } => aggregations,
        other => panic!("test: expected mixed columns, got {other:?}"),
    };
    let base = "FROM docs WHERE vector NEAR $v GROUP BY c";
    assert!(vector_group_score_is_average(&aggregations(&format!(
        "SELECT c, AVG(score) {base}"
    ))));
    assert!(!vector_group_score_is_average(&aggregations(&format!(
        "SELECT c, AVG(score), MAX(score) {base}"
    ))));
    assert!(!vector_group_score_is_average(&aggregations(&format!(
        "SELECT c, COUNT(*) {base}"
    ))));
}
