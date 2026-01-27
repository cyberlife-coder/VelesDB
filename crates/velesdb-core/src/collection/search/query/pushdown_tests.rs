//! Tests for `pushdown` module - Filter pushdown analysis.

use super::pushdown::*;
use crate::velesql::{
    ColumnRef, CompareOp, Comparison, Condition, JoinClause, JoinCondition, Value,
};
use std::collections::HashSet;

fn make_comparison(column: &str, value: i64) -> Condition {
    Condition::Comparison(Comparison {
        column: column.to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(value),
    })
}

fn make_graph_vars() -> HashSet<String> {
    let mut vars = HashSet::new();
    vars.insert("t".to_string());
    vars.insert("p".to_string());
    vars
}

fn make_join_tables() -> HashSet<String> {
    let mut tables = HashSet::new();
    tables.insert("prices".to_string());
    tables.insert("availability".to_string());
    tables
}

#[test]
fn test_pushdown_analysis_new() {
    let analysis = PushdownAnalysis::new();
    assert!(!analysis.has_pushdown());
    assert_eq!(analysis.total_conditions(), 0);
}

#[test]
fn test_classify_column_store_filter() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let condition = make_comparison("prices.price", 1000);
    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(analysis.has_pushdown());
    assert_eq!(analysis.column_store_filters.len(), 1);
    assert!(analysis.graph_filters.is_empty());
    assert!(analysis.post_join_filters.is_empty());
}

#[test]
fn test_classify_graph_filter() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let condition = make_comparison("t.category", 1);
    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(!analysis.has_pushdown());
    assert!(analysis.column_store_filters.is_empty());
    assert_eq!(analysis.graph_filters.len(), 1);
    assert!(analysis.post_join_filters.is_empty());
}

#[test]
fn test_classify_simple_column_defaults_to_graph() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let condition = make_comparison("category", 1);
    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(!analysis.has_pushdown());
    assert_eq!(analysis.graph_filters.len(), 1);
}

#[test]
fn test_classify_and_splits_conditions() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let graph_filter = make_comparison("t.category", 1);
    let column_filter = make_comparison("prices.price", 1000);
    let condition = Condition::And(Box::new(graph_filter), Box::new(column_filter));

    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(analysis.has_pushdown());
    assert_eq!(analysis.column_store_filters.len(), 1);
    assert_eq!(analysis.graph_filters.len(), 1);
    assert!(analysis.post_join_filters.is_empty());
}

#[test]
fn test_classify_or_keeps_together() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let graph_filter = make_comparison("t.category", 1);
    let column_filter = make_comparison("prices.price", 1000);
    let condition = Condition::Or(Box::new(graph_filter), Box::new(column_filter));

    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(!analysis.has_pushdown());
    assert!(analysis.column_store_filters.is_empty());
    assert!(analysis.graph_filters.is_empty());
    assert_eq!(analysis.post_join_filters.len(), 1);
}

#[test]
fn test_classify_or_same_source() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let filter1 = make_comparison("prices.price", 1000);
    let filter2 = make_comparison("prices.discount", 10);
    let condition = Condition::Or(Box::new(filter1), Box::new(filter2));

    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(analysis.has_pushdown());
    assert_eq!(analysis.column_store_filters.len(), 1);
}

#[test]
fn test_extract_join_tables() {
    let joins = vec![JoinClause {
        join_type: crate::velesql::JoinType::Inner,
        table: "prices".to_string(),
        alias: Some("pr".to_string()),
        condition: Some(JoinCondition {
            left: ColumnRef {
                table: Some("prices".to_string()),
                column: "trip_id".to_string(),
            },
            right: ColumnRef {
                table: Some("t".to_string()),
                column: "id".to_string(),
            },
        }),
        using_columns: None,
    }];

    let tables = extract_join_tables(&joins);

    assert!(tables.contains("prices"));
    assert!(tables.contains("pr"));
    assert_eq!(tables.len(), 2);
}

#[test]
fn test_complex_nested_and() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let inner_and = Condition::And(
        Box::new(make_comparison("t.a", 1)),
        Box::new(make_comparison("prices.b", 2)),
    );
    let condition = Condition::And(Box::new(inner_and), Box::new(make_comparison("t.c", 3)));

    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert_eq!(analysis.column_store_filters.len(), 1);
    assert_eq!(analysis.graph_filters.len(), 2);
}

#[test]
fn test_unknown_table_prefix_goes_to_post_join() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let condition = make_comparison("unknown_table.column", 1);
    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(!analysis.has_pushdown());
    assert!(analysis.column_store_filters.is_empty());
    assert!(analysis.graph_filters.is_empty());
    assert_eq!(analysis.post_join_filters.len(), 1);
}

#[test]
fn test_misspelled_table_goes_to_post_join() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let condition = make_comparison("price.value", 100);
    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(!analysis.has_pushdown());
    assert_eq!(analysis.post_join_filters.len(), 1);
}

#[test]
fn test_unknown_combined_with_known_stays_post_join() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let unknown_filter = make_comparison("unknown_table.x", 1);
    let known_filter = make_comparison("prices.y", 2);
    let condition = Condition::And(Box::new(unknown_filter), Box::new(known_filter));

    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert_eq!(analysis.column_store_filters.len(), 1);
    assert_eq!(analysis.post_join_filters.len(), 1);
}

#[test]
fn test_or_with_unknown_goes_to_post_join() {
    let graph_vars = make_graph_vars();
    let join_tables = make_join_tables();

    let unknown_filter = make_comparison("unknown_table.x", 1);
    let known_filter = make_comparison("prices.y", 2);
    let condition = Condition::Or(Box::new(unknown_filter), Box::new(known_filter));

    let analysis = analyze_for_pushdown(&condition, &graph_vars, &join_tables);

    assert!(!analysis.has_pushdown());
    assert!(analysis.column_store_filters.is_empty());
    assert_eq!(analysis.post_join_filters.len(), 1);
}
