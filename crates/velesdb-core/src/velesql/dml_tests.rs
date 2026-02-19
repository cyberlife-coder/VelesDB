//! Tests for INSERT/UPDATE parsing (VelesQL DML).

use crate::velesql::{DmlStatement, Parser, Value};

#[test]
fn test_parse_insert_statement() {
    let query = Parser::parse("INSERT INTO products (id, name, price) VALUES (1, 'Pen', 2.5)")
        .expect("INSERT should parse");
    let dml = query.dml.expect("Expected DML statement");

    match dml {
        DmlStatement::Insert(insert) => {
            assert_eq!(insert.table, "products");
            assert_eq!(insert.columns, vec!["id", "name", "price"]);
            assert_eq!(insert.values.len(), 3);
            assert_eq!(insert.values[0], Value::Integer(1));
            assert_eq!(insert.values[1], Value::String("Pen".to_string()));
        }
        DmlStatement::Update(_) => panic!("Expected INSERT statement"),
    }
}

#[test]
fn test_parse_update_statement_with_where() {
    let query = Parser::parse("UPDATE products SET price = 3.0, active = true WHERE id = 1")
        .expect("UPDATE should parse");
    let dml = query.dml.expect("Expected DML statement");

    match dml {
        DmlStatement::Update(update) => {
            assert_eq!(update.table, "products");
            assert_eq!(update.assignments.len(), 2);
            assert!(update.where_clause.is_some());
            assert_eq!(update.assignments[0].column, "price");
            assert_eq!(update.assignments[1].column, "active");
        }
        DmlStatement::Insert(_) => panic!("Expected UPDATE statement"),
    }
}
