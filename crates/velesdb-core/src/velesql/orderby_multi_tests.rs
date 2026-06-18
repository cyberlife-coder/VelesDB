//! Tests for ORDER BY multi-expression support (EPIC-040 US-002, EPIC-042).
//!
//! Covers:
//! - ORDER BY multiple columns
//! - ORDER BY with aggregate functions
//! - ORDER BY mixed (columns + aggregates)
//! - Direction per column (ASC/DESC)
//! - ORDER BY arithmetic expressions (EPIC-042)

use crate::velesql::{
    AggregateArg, AggregateType, ArithmeticExpr, ArithmeticOp, OrderByExpr, Parser,
};

#[test]
fn test_orderby_multiple_columns() {
    let sql = "SELECT * FROM products ORDER BY category ASC, price DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 2, "Should have 2 ORDER BY items");
    // First: category ASC
    assert!(matches!(&order_by[0].expr, OrderByExpr::Field(f) if f == "category"));
    assert!(!order_by[0].descending, "category should be ASC");
    // Second: price DESC
    assert!(matches!(&order_by[1].expr, OrderByExpr::Field(f) if f == "price"));
    assert!(order_by[1].descending, "price should be DESC");
}

#[test]
fn test_orderby_with_aggregate() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category ORDER BY COUNT(*) DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 1);
    assert!(order_by[0].descending, "Should be DESC");
    // The expression should represent COUNT(*)
    assert!(
        matches!(&order_by[0].expr, OrderByExpr::Aggregate(_)),
        "Should be Aggregate variant"
    );
}

#[test]
fn test_orderby_mixed_columns_and_aggregates() {
    let sql = "SELECT category, COUNT(*), AVG(price) FROM products GROUP BY category ORDER BY COUNT(*) DESC, category ASC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 2, "Should have 2 ORDER BY items");
    // First: COUNT(*) DESC
    assert!(order_by[0].descending);
    assert!(
        matches!(&order_by[0].expr, OrderByExpr::Aggregate(_)),
        "First expr should be Aggregate (COUNT(*)), got {:?}",
        order_by[0].expr
    );
    // Second: category ASC
    assert!(!order_by[1].descending);
    assert!(
        matches!(&order_by[1].expr, OrderByExpr::Field(f) if f == "category"),
        "Second expr should be Field(category), got {:?}",
        order_by[1].expr
    );
}

#[test]
fn test_orderby_aggregate_with_column_arg() {
    let sql =
        "SELECT category, SUM(price) FROM products GROUP BY category ORDER BY SUM(price) DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 1);
    assert!(order_by[0].descending, "Should be DESC");
    match &order_by[0].expr {
        OrderByExpr::Aggregate(agg) => {
            assert_eq!(
                agg.function_type,
                AggregateType::Sum,
                "Expected SUM aggregate"
            );
            assert_eq!(
                agg.argument,
                AggregateArg::Column("price".to_string()),
                "Expected column argument 'price'"
            );
        }
        other => panic!("Expected Aggregate(SUM(price)), got {other:?}"),
    }
}

#[test]
fn test_orderby_default_direction_is_asc() {
    let sql = "SELECT * FROM products ORDER BY price, category";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 2);
    // Both should default to ASC (descending = false)
    assert!(matches!(&order_by[0].expr, OrderByExpr::Field(f) if f == "price"));
    assert!(!order_by[0].descending, "Default should be ASC");
    assert!(matches!(&order_by[1].expr, OrderByExpr::Field(f) if f == "category"));
    assert!(!order_by[1].descending, "Default should be ASC");
}

#[test]
fn test_orderby_similarity_with_column() {
    let sql = "SELECT * FROM products ORDER BY similarity(embedding, $query) DESC, price ASC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 2);
    assert!(matches!(&order_by[0].expr, OrderByExpr::Similarity(_)));
    assert!(order_by[0].descending, "similarity should be DESC");
    assert!(matches!(&order_by[1].expr, OrderByExpr::Field(f) if f == "price"));
    assert!(!order_by[1].descending, "price should be ASC");
}

// -----------------------------------------------------------------------
// EPIC-042: Arithmetic ORDER BY expressions
// -----------------------------------------------------------------------

#[test]
fn test_parse_order_by_simple_arithmetic() {
    let sql = "SELECT * FROM docs ORDER BY 0.7 * vector_score + 0.3 * graph_score DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");
    assert_eq!(order_by.len(), 1);
    assert!(order_by[0].descending, "Should be DESC");

    // Verify structure: (0.7 * vector_score) + (0.3 * graph_score)
    match &order_by[0].expr {
        OrderByExpr::Arithmetic(ArithmeticExpr::BinaryOp { left, op, right }) => {
            assert_eq!(*op, ArithmeticOp::Add);
            // Left: 0.7 * vector_score
            match left.as_ref() {
                ArithmeticExpr::BinaryOp {
                    left: ll,
                    op: lo,
                    right: lr,
                } => {
                    assert_eq!(*lo, ArithmeticOp::Mul);
                    assert!(
                        matches!(ll.as_ref(), ArithmeticExpr::Literal(v) if (*v - 0.7).abs() < 1e-9)
                    );
                    assert!(
                        matches!(lr.as_ref(), ArithmeticExpr::Variable(n) if n == "vector_score")
                    );
                }
                other => panic!("Expected BinaryOp(Mul), got {other:?}"),
            }
            // Right: 0.3 * graph_score
            match right.as_ref() {
                ArithmeticExpr::BinaryOp {
                    left: rl,
                    op: ro,
                    right: rr,
                } => {
                    assert_eq!(*ro, ArithmeticOp::Mul);
                    assert!(
                        matches!(rl.as_ref(), ArithmeticExpr::Literal(v) if (*v - 0.3).abs() < 1e-9)
                    );
                    assert!(
                        matches!(rr.as_ref(), ArithmeticExpr::Variable(n) if n == "graph_score")
                    );
                }
                other => panic!("Expected BinaryOp(Mul), got {other:?}"),
            }
        }
        other => panic!("Expected Arithmetic, got {other:?}"),
    }
}

#[test]
fn test_parse_order_by_parenthesized() {
    let sql = "SELECT * FROM docs ORDER BY (vector_score + graph_score) / 2 DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");
    assert_eq!(order_by.len(), 1);
    assert!(order_by[0].descending);

    // Verify structure: (vector_score + graph_score) / 2
    match &order_by[0].expr {
        OrderByExpr::Arithmetic(ArithmeticExpr::BinaryOp { left, op, right }) => {
            assert_eq!(*op, ArithmeticOp::Div);
            // Left: (vector_score + graph_score)
            match left.as_ref() {
                ArithmeticExpr::BinaryOp {
                    left: ll,
                    op: lo,
                    right: lr,
                } => {
                    assert_eq!(*lo, ArithmeticOp::Add);
                    assert!(
                        matches!(ll.as_ref(), ArithmeticExpr::Variable(n) if n == "vector_score")
                    );
                    assert!(
                        matches!(lr.as_ref(), ArithmeticExpr::Variable(n) if n == "graph_score")
                    );
                }
                other => panic!("Expected BinaryOp(Add), got {other:?}"),
            }
            // Right: 2
            assert!(
                matches!(right.as_ref(), ArithmeticExpr::Literal(v) if (*v - 2.0).abs() < 1e-9)
            );
        }
        other => panic!("Expected Arithmetic, got {other:?}"),
    }
}

#[test]
fn test_parse_order_by_single_variable_remains_field() {
    // Backward compatibility: a single identifier should still be Field, not Arithmetic.
    let sql = "SELECT * FROM products ORDER BY price ASC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");
    assert_eq!(order_by.len(), 1);
    assert!(!order_by[0].descending, "Should be ASC");
    assert!(
        matches!(&order_by[0].expr, OrderByExpr::Field(f) if f == "price"),
        "Single identifier must collapse to Field, got {:?}",
        order_by[0].expr
    );
}

#[test]
fn test_parse_order_by_similarity_bare_in_arithmetic() {
    let sql =
        "SELECT * FROM docs WHERE vector NEAR $v ORDER BY 0.5 * similarity() + 0.5 * bm25_score DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");
    assert_eq!(order_by.len(), 1);
    assert!(order_by[0].descending);

    // Verify structure: (0.5 * similarity()) + (0.5 * bm25_score)
    match &order_by[0].expr {
        OrderByExpr::Arithmetic(ArithmeticExpr::BinaryOp { left, op, right }) => {
            assert_eq!(*op, ArithmeticOp::Add);
            // Left: 0.5 * similarity()
            match left.as_ref() {
                ArithmeticExpr::BinaryOp {
                    left: ll,
                    op: lo,
                    right: lr,
                } => {
                    assert_eq!(*lo, ArithmeticOp::Mul);
                    assert!(
                        matches!(ll.as_ref(), ArithmeticExpr::Literal(v) if (*v - 0.5).abs() < 1e-9)
                    );
                    assert!(
                        matches!(lr.as_ref(), ArithmeticExpr::Similarity(inner) if **inner == OrderByExpr::SimilarityBare),
                        "Expected Similarity(SimilarityBare), got {:?}",
                        lr
                    );
                }
                other => panic!("Expected BinaryOp(Mul), got {other:?}"),
            }
            // Right: 0.5 * bm25_score
            match right.as_ref() {
                ArithmeticExpr::BinaryOp {
                    left: rl,
                    op: ro,
                    right: rr,
                } => {
                    assert_eq!(*ro, ArithmeticOp::Mul);
                    assert!(
                        matches!(rl.as_ref(), ArithmeticExpr::Literal(v) if (*v - 0.5).abs() < 1e-9)
                    );
                    assert!(
                        matches!(rr.as_ref(), ArithmeticExpr::Variable(n) if n == "bm25_score")
                    );
                }
                other => panic!("Expected BinaryOp(Mul), got {other:?}"),
            }
        }
        other => panic!("Expected Arithmetic, got {other:?}"),
    }
}

#[test]
fn test_parse_order_by_similarity_bare_alone_remains_bare() {
    // Backward compatibility: ORDER BY similarity() DESC should still be SimilarityBare.
    let sql = "SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");
    assert_eq!(order_by.len(), 1);
    assert!(order_by[0].descending);
    assert_eq!(
        order_by[0].expr,
        OrderByExpr::SimilarityBare,
        "Bare similarity() must collapse to SimilarityBare, got {:?}",
        order_by[0].expr,
    );
}

#[test]
fn test_parse_order_by_literal_only() {
    // A single numeric literal in ORDER BY is a valid arithmetic expression.
    let sql = "SELECT * FROM docs ORDER BY 42 DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");
    assert_eq!(order_by.len(), 1);
    assert!(
        matches!(&order_by[0].expr, OrderByExpr::Arithmetic(ArithmeticExpr::Literal(v)) if (*v - 42.0).abs() < 1e-9),
        "Expected Arithmetic(Literal(42)), got {:?}",
        order_by[0].expr
    );
}

#[test]
fn test_parse_order_by_subtraction() {
    let sql = "SELECT * FROM docs ORDER BY vector_score - penalty DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");
    assert_eq!(order_by.len(), 1);
    match &order_by[0].expr {
        OrderByExpr::Arithmetic(ArithmeticExpr::BinaryOp { op, .. }) => {
            assert_eq!(*op, ArithmeticOp::Sub);
        }
        other => panic!("Expected Arithmetic(BinaryOp(Sub)), got {other:?}"),
    }
}
