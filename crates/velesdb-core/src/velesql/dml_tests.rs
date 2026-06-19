//! Tests for INSERT/UPDATE parsing (VelesQL DML).
//!
//! Also covers collection name resolution across all DML variants
//! (regression tests from Devin review).

use crate::velesql::{CompareOp, Condition, DmlStatement, Parser, Value};

#[test]
fn test_parse_insert_statement() {
    let query = Parser::parse("INSERT INTO products (id, name, price) VALUES (1, 'Pen', 2.5)")
        .expect("INSERT should parse");
    let dml = query.dml.expect("Expected DML statement");

    match dml {
        DmlStatement::Insert(insert) => {
            assert_eq!(insert.table, "products");
            assert_eq!(insert.columns, vec!["id", "name", "price"]);
            assert_eq!(insert.rows.len(), 1);
            assert_eq!(insert.rows[0].len(), 3);
            assert_eq!(insert.rows[0][0], Value::Integer(1));
            assert_eq!(insert.rows[0][1], Value::String("Pen".to_string()));
        }
        DmlStatement::Update(_) => panic!("Expected INSERT statement"),
        _ => panic!("Unexpected DML variant"),
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
            assert_eq!(update.assignments[0].column, "price");
            assert_eq!(update.assignments[0].value, Value::Float(3.0));
            assert_eq!(update.assignments[1].column, "active");
            assert_eq!(update.assignments[1].value, Value::Boolean(true));
            let where_clause = update.where_clause.expect("Expected WHERE clause");
            let Condition::Comparison(cmp) = where_clause else {
                panic!("Expected Comparison condition");
            };
            assert_eq!(cmp.column, "id");
            assert_eq!(cmp.operator, CompareOp::Eq);
            assert_eq!(cmp.value, Value::Integer(1));
        }
        DmlStatement::Insert(_) => panic!("Expected UPDATE statement"),
        _ => panic!("Unexpected DML variant"),
    }
}

// ============================================================================
// Regression — DML collection name resolution (Devin review)
// ============================================================================

/// INSERT INTO must set the `table` field to the collection name.
#[test]
fn test_dml_collection_name_insert() {
    let query = Parser::parse("INSERT INTO products (id) VALUES (1)")
        .expect("test: INSERT INTO should parse");

    let dml = query.dml.expect("test: expected DML statement");
    let DmlStatement::Insert(insert) = dml else {
        panic!("Expected Insert variant, got {dml:?}");
    };

    assert_eq!(insert.table, "products");
}

/// DELETE FROM must set the `table` field to the collection name.
#[test]
fn test_dml_collection_name_delete() {
    let query =
        Parser::parse("DELETE FROM products WHERE id = 1").expect("test: DELETE FROM should parse");

    let dml = query.dml.expect("test: expected DML statement");
    let DmlStatement::Delete(delete) = dml else {
        panic!("Expected Delete variant, got {dml:?}");
    };

    assert_eq!(delete.table, "products");
}

/// INSERT EDGE INTO must set the `collection` field to the graph collection name.
#[test]
fn test_dml_collection_name_insert_edge() {
    let query = Parser::parse("INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS')")
        .expect("test: INSERT EDGE INTO should parse");

    let dml = query.dml.expect("test: expected DML statement");
    let DmlStatement::InsertEdge(edge) = dml else {
        panic!("Expected InsertEdge variant, got {dml:?}");
    };

    assert_eq!(edge.collection, "kg");
}

/// SELECT EDGES FROM must set the `collection` field to the graph collection name.
#[test]
fn test_dml_collection_name_select_edges() {
    let query =
        Parser::parse("SELECT EDGES FROM kg").expect("test: SELECT EDGES FROM should parse");

    let dml = query.dml.expect("test: expected DML statement");
    let DmlStatement::SelectEdges(edges) = dml else {
        panic!("Expected SelectEdges variant, got {dml:?}");
    };

    assert_eq!(edges.collection, "kg");
}

/// INSERT NODE INTO must set the `collection` field to the graph collection name.
#[test]
fn test_dml_collection_name_insert_node() {
    let query = Parser::parse("INSERT NODE INTO kg (id = 1, payload = '{\"name\": \"A\"}')")
        .expect("test: INSERT NODE INTO should parse");

    let dml = query.dml.expect("test: expected DML statement");
    let DmlStatement::InsertNode(node) = dml else {
        panic!("Expected InsertNode variant, got {dml:?}");
    };

    assert_eq!(node.collection, "kg");
}
