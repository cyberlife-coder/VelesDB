use super::Database;
use crate::velesql::{Condition, Parser, Value};
use std::collections::HashMap;

#[test]
fn joined_time_filters_share_one_bound_before_pushdown() {
    let sql = "SELECT * FROM products p JOIN inventory i ON p.id = i.id \
               WHERE p.ts >= NOW() AND i.ts >= NOW() AND (p.ts < NOW() OR i.ts < NOW())";
    let parse = |sql: &str| Parser::parse(sql).expect("test: joined predicates");
    let mut query = parse(sql);
    let (analysis, filters) = Database::prepare_join_pushdown(&mut query, &HashMap::new()).unwrap();
    let Some(Condition::Comparison(base)) = &query.select.where_clause else {
        panic!("expected one predicate on the base");
    };
    let Value::Integer(bound) = base.value else {
        panic!("the base time bound was not frozen");
    };
    let mut control = parse(&sql.replace("NOW()", &bound.to_string()));
    let (expected, residual) =
        Database::prepare_join_pushdown(&mut control, &HashMap::new()).unwrap();
    assert!(!expected.column_store_filters.is_empty());
    assert!(!expected.post_join_filters.is_empty());
    assert_eq!(query.select.where_clause, control.select.where_clause);
    assert_eq!(analysis.column_store_filters, expected.column_store_filters);
    assert_eq!(filters, residual);
}
