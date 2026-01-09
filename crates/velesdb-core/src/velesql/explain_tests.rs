//! Tests for `explain` module

use super::ast::{
    CompareOp, Comparison, Condition, SelectColumns, SelectStatement, Value, VectorExpr,
    VectorSearch as VsCondition,
};
use super::explain::*;

#[test]
fn test_plan_from_simple_select() {
    // Arrange
    let stmt = SelectStatement {
        columns: SelectColumns::All,
        from: "documents".to_string(),
        where_clause: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);

    // Assert
    assert!(plan.index_used.is_none());
    assert_eq!(plan.filter_strategy, FilterStrategy::None);
    assert!(plan.estimated_cost_ms > 0.0);
}

#[test]
fn test_plan_from_vector_search() {
    // Arrange
    let stmt = SelectStatement {
        columns: SelectColumns::All,
        from: "embeddings".to_string(),
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("query".to_string()),
        })),
        limit: Some(5),
        offset: None,
        with_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);

    // Assert
    assert_eq!(plan.index_used, Some(IndexType::Hnsw));
    assert!(plan.estimated_cost_ms < 1.0);
}

#[test]
fn test_plan_with_filter() {
    // Arrange
    let stmt = SelectStatement {
        columns: SelectColumns::All,
        from: "docs".to_string(),
        where_clause: Some(Condition::And(
            Box::new(Condition::VectorSearch(VsCondition {
                vector: VectorExpr::Parameter("v".to_string()),
            })),
            Box::new(Condition::Comparison(Comparison {
                column: "category".to_string(),
                operator: CompareOp::Eq,
                value: Value::String("tech".to_string()),
            })),
        )),
        limit: Some(10),
        offset: None,
        with_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);

    // Assert
    assert_eq!(plan.index_used, Some(IndexType::Hnsw));
    assert_ne!(plan.filter_strategy, FilterStrategy::None);
}

#[test]
fn test_plan_to_tree_format() {
    // Arrange
    let stmt = SelectStatement {
        columns: SelectColumns::All,
        from: "documents".to_string(),
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("q".to_string()),
        })),
        limit: Some(10),
        offset: None,
        with_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();

    // Assert
    assert!(tree.contains("Query Plan:"));
    assert!(tree.contains("VectorSearch"));
    assert!(tree.contains("Collection: documents"));
    assert!(tree.contains("Index used: HNSW"));
}

#[test]
fn test_plan_to_json() {
    // Arrange
    let stmt = SelectStatement {
        columns: SelectColumns::All,
        from: "test".to_string(),
        where_clause: None,
        limit: Some(5),
        offset: None,
        with_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);
    let json = plan.to_json().expect("JSON serialization should succeed");

    // Assert
    assert!(json.contains("\"estimated_cost_ms\""));
    assert!(json.contains("\"root\""));
}

#[test]
fn test_plan_with_offset() {
    // Arrange
    let stmt = SelectStatement {
        columns: SelectColumns::All,
        from: "items".to_string(),
        where_clause: None,
        limit: Some(10),
        offset: Some(20),
        with_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();

    // Assert
    assert!(tree.contains("Offset: 20"));
    assert!(tree.contains("Limit: 10"));
}

#[test]
fn test_filter_strategy_post_filter_default() {
    // Arrange: Single filter condition = 50% selectivity = post-filter
    let stmt = SelectStatement {
        columns: SelectColumns::All,
        from: "docs".to_string(),
        where_clause: Some(Condition::And(
            Box::new(Condition::VectorSearch(VsCondition {
                vector: VectorExpr::Parameter("v".to_string()),
            })),
            Box::new(Condition::Comparison(Comparison {
                column: "status".to_string(),
                operator: CompareOp::Eq,
                value: Value::String("active".to_string()),
            })),
        )),
        limit: Some(10),
        offset: None,
        with_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);

    // Assert
    assert_eq!(plan.filter_strategy, FilterStrategy::PostFilter);
}

#[test]
fn test_index_type_as_str() {
    assert_eq!(IndexType::Hnsw.as_str(), "HNSW");
    assert_eq!(IndexType::Flat.as_str(), "Flat");
    assert_eq!(IndexType::BinaryQuantization.as_str(), "BinaryQuantization");
}

#[test]
fn test_compare_op_as_str() {
    assert_eq!(CompareOp::Eq.as_str(), "=");
    assert_eq!(CompareOp::NotEq.as_str(), "!=");
    assert_eq!(CompareOp::Gt.as_str(), ">");
    assert_eq!(CompareOp::Gte.as_str(), ">=");
    assert_eq!(CompareOp::Lt.as_str(), "<");
    assert_eq!(CompareOp::Lte.as_str(), "<=");
}

#[test]
fn test_plan_display_impl() {
    // Arrange
    let stmt = SelectStatement {
        columns: SelectColumns::All,
        from: "test".to_string(),
        where_clause: None,
        limit: Some(5),
        offset: None,
        with_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);
    let display = format!("{plan}");

    // Assert
    assert!(display.contains("Query Plan:"));
}
