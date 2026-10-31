//! Tests for [`super::grouped_order`]: which column of a grouped row an
//! ORDER BY item sorts on, and the `V016` refusal of one that names none.

use super::ast::{OrderByExpr, SelectColumns, SelectStatement};
use super::validation::{QueryValidator, ValidationError, ValidationErrorKind};
use super::{vector_group_score_is_average, Parser};

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

fn assert_v017(sql: &str) {
    let err = validate(sql).expect_err("test: duplicate output column must be refused");
    assert_eq!(
        err.kind,
        ValidationErrorKind::DuplicateOutputColumn,
        "{err}"
    );
    assert_eq!(err.kind.code(), "V017");
}

#[test]
fn test_validator_refuses_a_group_column_sharing_an_aggregates_default_name() {
    assert_v017("SELECT category, COUNT(*) FROM docs GROUP BY category, count");
}

#[test]
fn test_validator_refuses_two_aggregates_sharing_an_alias() {
    assert_v017("SELECT category, COUNT(*) AS n, SUM(year) AS n FROM docs GROUP BY category");
}

#[test]
fn test_validator_refuses_an_alias_shadowing_a_group_column() {
    assert_v017("SELECT category, COUNT(*) AS category FROM docs GROUP BY category");
}

#[test]
fn test_validator_refuses_duplicate_names_without_group_by_and_in_a_vector_group_by() {
    assert_v017("SELECT COUNT(*), COUNT(*) FROM docs");
    assert_v017(
        "SELECT category, MAX(score) AS category FROM docs WHERE vector NEAR $v GROUP BY category",
    );
}

#[test]
fn test_validator_accepts_a_selected_group_column_and_distinct_aggregate_names() {
    validate("SELECT category, category, COUNT(*), SUM(year) AS year FROM docs GROUP BY category")
        .expect("test: a group column listed twice is one column; `year` is not output");
}

// ---------------------------------------------------------------------------
// grouped_sort_keys: the field each ORDER BY item sorts on
// ---------------------------------------------------------------------------

fn select(sql: &str) -> SelectStatement {
    Parser::parse(sql).expect("test: parse").select
}

fn sort_keys(sql: &str) -> Vec<(String, bool)> {
    select(sql).grouped_sort_keys().expect("test: resolve")
}

#[test]
fn test_a_name_resolves_to_an_alias_a_default_name_or_a_group_column() {
    assert_eq!(
        sort_keys(
            "SELECT category, COUNT(*), SUM(year) AS total FROM docs GROUP BY category \
             ORDER BY total DESC, count, category"
        ),
        vec![
            ("total".to_string(), true),
            ("count".to_string(), false),
            ("category".to_string(), false),
        ]
    );
}

#[test]
fn test_a_default_name_never_matches_an_aliased_aggregate() {
    let stmt = select("SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY count");
    assert_eq!(
        stmt.grouped_sort_keys()
            .expect_err("test: refused")
            .kind
            .code(),
        "V016"
    );
}

#[test]
fn test_a_repeated_aggregate_resolves_to_its_select_items_field_whatever_the_alias() {
    assert_eq!(
        sort_keys(
            "SELECT category, AVG(year) AS mean, SUM(year) FROM docs GROUP BY category \
             ORDER BY AVG(year) DESC, SUM(year)"
        ),
        vec![("mean".to_string(), true), ("sum_year".to_string(), false)]
    );
}

// ---------------------------------------------------------------------------
// order_by_aggregates_as_fields and the vector GROUP BY score
// ---------------------------------------------------------------------------

#[test]
fn test_order_by_aggregates_become_their_output_field_and_the_rest_is_kept() {
    let stmt = select(
        "SELECT category, MAX(score) AS best FROM docs WHERE vector NEAR $v \
         GROUP BY category ORDER BY MAX(score) DESC, similarity() ASC",
    );
    let rewritten = stmt.order_by_aggregates_as_fields().expect("test: rewrite");
    let order_by = rewritten.order_by.as_deref().expect("test: order by");
    assert_eq!(order_by[0].expr, OrderByExpr::Field("best".to_string()));
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
        .order_by_aggregates_as_fields()
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
