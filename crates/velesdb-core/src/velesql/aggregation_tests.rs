//! Tests for `VelesQL` aggregation parsing (EPIC-017 US-001).

use crate::velesql::{AggregateArg, AggregateType, Parser, SelectColumns};

#[test]
fn test_parser_count_star() {
    let query = Parser::parse("SELECT COUNT(*) FROM documents").unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 1);
            assert_eq!(aggs[0].function_type, AggregateType::Count);
            assert_eq!(aggs[0].argument, AggregateArg::Wildcard);
            assert!(aggs[0].alias.is_none());
        }
        _ => panic!("Expected Aggregations, got {:?}", query.select.columns),
    }
}

#[test]
fn test_parser_count_column() {
    let query = Parser::parse("SELECT COUNT(id) FROM documents").unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 1);
            assert_eq!(aggs[0].function_type, AggregateType::Count);
            assert_eq!(aggs[0].argument, AggregateArg::Column("id".to_string()));
        }
        _ => panic!("Expected Aggregations"),
    }
}

#[test]
fn test_parser_sum_avg_min_max() {
    let query =
        Parser::parse("SELECT SUM(price), AVG(rating), MIN(created), MAX(updated) FROM items")
            .unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 4);

            assert_eq!(aggs[0].function_type, AggregateType::Sum);
            assert_eq!(aggs[0].argument, AggregateArg::Column("price".to_string()));

            assert_eq!(aggs[1].function_type, AggregateType::Avg);
            assert_eq!(aggs[1].argument, AggregateArg::Column("rating".to_string()));

            assert_eq!(aggs[2].function_type, AggregateType::Min);
            assert_eq!(
                aggs[2].argument,
                AggregateArg::Column("created".to_string())
            );

            assert_eq!(aggs[3].function_type, AggregateType::Max);
            assert_eq!(
                aggs[3].argument,
                AggregateArg::Column("updated".to_string())
            );
        }
        _ => panic!("Expected Aggregations"),
    }
}

#[test]
fn test_parser_aggregation_with_alias() {
    let query = Parser::parse("SELECT COUNT(*) AS total FROM documents").unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 1);
            assert_eq!(aggs[0].function_type, AggregateType::Count);
            assert_eq!(aggs[0].alias, Some("total".to_string()));
        }
        _ => panic!("Expected Aggregations"),
    }
}

#[test]
fn test_parser_multiple_aggregations_with_aliases() {
    let query =
        Parser::parse("SELECT COUNT(*) AS cnt, AVG(score) AS avg_score FROM results").unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 2);

            assert_eq!(aggs[0].function_type, AggregateType::Count);
            assert_eq!(aggs[0].alias, Some("cnt".to_string()));

            assert_eq!(aggs[1].function_type, AggregateType::Avg);
            assert_eq!(aggs[1].alias, Some("avg_score".to_string()));
        }
        _ => panic!("Expected Aggregations"),
    }
}

#[test]
fn test_parser_aggregation_case_insensitive() {
    let query = Parser::parse("select count(*) from docs").unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 1);
            assert_eq!(aggs[0].function_type, AggregateType::Count);
        }
        _ => panic!("Expected Aggregations"),
    }
}

#[test]
fn test_parser_aggregation_with_where_clause() {
    let query = Parser::parse("SELECT COUNT(*) FROM documents WHERE category = 'tech'").unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 1);
            assert_eq!(aggs[0].function_type, AggregateType::Count);
        }
        _ => panic!("Expected Aggregations"),
    }

    assert!(query.select.where_clause.is_some());
}

#[test]
fn test_parser_aggregation_with_limit() {
    let query = Parser::parse("SELECT COUNT(*) FROM documents LIMIT 1").unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 1);
        }
        _ => panic!("Expected Aggregations"),
    }

    assert_eq!(query.select.limit, Some(1));
}
