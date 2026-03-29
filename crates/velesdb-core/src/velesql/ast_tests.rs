//! Tests for `ast` module

use super::ast::*;

#[test]
fn test_column_new() {
    let col = Column::new("id");
    assert_eq!(col.name, "id");
    assert!(col.alias.is_none());
}

#[test]
fn test_column_with_alias() {
    let col = Column::with_alias("payload.title", "title");
    assert_eq!(col.name, "payload.title");
    assert_eq!(col.alias, Some("title".to_string()));
}

#[test]
fn test_value_from_integer() {
    let v: Value = 42i64.into();
    assert_eq!(v, Value::Integer(42));
}

#[test]
fn test_value_from_float() {
    let v: Value = 2.5f64.into();
    assert_eq!(v, Value::Float(2.5));
}

#[test]
fn test_value_from_string() {
    let v: Value = "hello".into();
    assert_eq!(v, Value::String("hello".to_string()));
}

#[test]
fn test_value_from_bool() {
    let v: Value = true.into();
    assert_eq!(v, Value::Boolean(true));
}

#[test]
fn test_query_serialization() {
    let query = Query {
        let_bindings: vec![],
        compound: None,
        match_clause: None,
        dml: None,
        train: None,
        select: SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: "documents".to_string(),
            from_alias: vec![],
            joins: vec![],
            where_clause: None,
            order_by: None,
            limit: Some(10),
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
    };

    let json = serde_json::to_string(&query).unwrap();
    let parsed: Query = serde_json::from_str(&json).unwrap();
    assert_eq!(query, parsed);
}

// ============================================================================
// Bug 4 regression: Display for ArithmeticOp and ArithmeticExpr
// ============================================================================

#[test]
fn test_arithmetic_op_display_add() {
    assert_eq!(format!("{}", ArithmeticOp::Add), "+");
}

#[test]
fn test_arithmetic_op_display_sub() {
    assert_eq!(format!("{}", ArithmeticOp::Sub), "-");
}

#[test]
fn test_arithmetic_op_display_mul() {
    assert_eq!(format!("{}", ArithmeticOp::Mul), "*");
}

#[test]
fn test_arithmetic_op_display_div() {
    assert_eq!(format!("{}", ArithmeticOp::Div), "/");
}

#[test]
fn test_arithmetic_expr_display_literal() {
    let expr = ArithmeticExpr::Literal(0.7);
    assert_eq!(format!("{expr}"), "0.7");
}

#[test]
fn test_arithmetic_expr_display_variable() {
    let expr = ArithmeticExpr::Variable("vector_score".to_string());
    assert_eq!(format!("{expr}"), "vector_score");
}

#[test]
fn test_arithmetic_expr_display_similarity_bare() {
    let expr = ArithmeticExpr::Similarity(Box::new(OrderByExpr::SimilarityBare));
    assert_eq!(format!("{expr}"), "similarity()");
}

#[test]
fn test_arithmetic_expr_display_binary_op() {
    let expr = ArithmeticExpr::BinaryOp {
        left: Box::new(ArithmeticExpr::Literal(0.7)),
        op: ArithmeticOp::Mul,
        right: Box::new(ArithmeticExpr::Variable("vector_score".to_string())),
    };
    assert_eq!(format!("{expr}"), "(0.7 * vector_score)");
}

#[test]
fn test_arithmetic_expr_display_complex() {
    // 0.7 * vector_score + 0.3 * graph_score
    let expr = ArithmeticExpr::BinaryOp {
        left: Box::new(ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::Literal(0.7)),
            op: ArithmeticOp::Mul,
            right: Box::new(ArithmeticExpr::Variable("vector_score".to_string())),
        }),
        op: ArithmeticOp::Add,
        right: Box::new(ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::Literal(0.3)),
            op: ArithmeticOp::Mul,
            right: Box::new(ArithmeticExpr::Variable("graph_score".to_string())),
        }),
    };
    assert_eq!(
        format!("{expr}"),
        "((0.7 * vector_score) + (0.3 * graph_score))"
    );
}

#[test]
fn test_arithmetic_expr_display_similarity_parameterized() {
    let expr = ArithmeticExpr::Similarity(Box::new(OrderByExpr::Similarity(SimilarityOrderBy {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("alt_vec".to_string()),
    })));
    assert_eq!(format!("{expr}"), "similarity(embedding, $alt_vec)");
}
