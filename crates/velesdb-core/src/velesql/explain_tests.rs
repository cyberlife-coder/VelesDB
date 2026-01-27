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
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "documents".to_string(),
        from_alias: None,
        joins: vec![],
        where_clause: None,
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
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
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "embeddings".to_string(),
        from_alias: None,
        joins: vec![],
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("query".to_string()),
        })),
        order_by: None,
        limit: Some(5),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
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
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: None,
        joins: vec![],
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
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
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
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "documents".to_string(),
        from_alias: None,
        joins: vec![],
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("q".to_string()),
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
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
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "test".to_string(),
        from_alias: None,
        joins: vec![],
        where_clause: None,
        order_by: None,
        limit: Some(5),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
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
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "items".to_string(),
        from_alias: None,
        joins: vec![],
        where_clause: None,
        order_by: None,
        limit: Some(10),
        offset: Some(20),
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
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
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: None,
        joins: vec![],
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
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
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
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "test".to_string(),
        from_alias: None,
        joins: vec![],
        where_clause: None,
        order_by: None,
        limit: Some(5),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    // Act
    let plan = QueryPlan::from_select(&stmt);
    let display = format!("{plan}");

    // Assert
    assert!(display.contains("Query Plan:"));
}

// =========================================================================
// IndexLookup tests (US-003)
// =========================================================================

#[test]
fn test_index_lookup_plan_creation() {
    // Arrange
    let plan = IndexLookupPlan {
        label: "Person".to_string(),
        property: "email".to_string(),
        value: "alice@example.com".to_string(),
    };

    // Assert
    assert_eq!(plan.label, "Person");
    assert_eq!(plan.property, "email");
    assert_eq!(plan.value, "alice@example.com");
}

#[test]
fn test_index_lookup_node_cost() {
    // IndexLookup should have very low cost (O(1))
    let plan = QueryPlan {
        root: PlanNode::IndexLookup(IndexLookupPlan {
            label: "Person".to_string(),
            property: "email".to_string(),
            value: "test@test.com".to_string(),
        }),
        estimated_cost_ms: 0.0001,
        index_used: Some(IndexType::Property),
        filter_strategy: FilterStrategy::None,
    };

    // IndexLookup cost should be much lower than TableScan
    let scan_plan = QueryPlan {
        root: PlanNode::TableScan(TableScanPlan {
            collection: "Person".to_string(),
        }),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
    };

    assert!(plan.estimated_cost_ms < scan_plan.estimated_cost_ms);
}

#[test]
fn test_index_lookup_render_tree() {
    // Arrange
    let plan = QueryPlan {
        root: PlanNode::IndexLookup(IndexLookupPlan {
            label: "Person".to_string(),
            property: "email".to_string(),
            value: "alice@example.com".to_string(),
        }),
        estimated_cost_ms: 0.0001,
        index_used: Some(IndexType::Property),
        filter_strategy: FilterStrategy::None,
    };

    // Act
    let tree = plan.to_tree();

    // Assert - EXPLAIN should show IndexLookup(Person.email)
    assert!(tree.contains("IndexLookup(Person.email)"));
    assert!(tree.contains("Value: alice@example.com"));
    assert!(tree.contains("Index used: PropertyIndex"));
}

#[test]
fn test_index_type_property() {
    assert_eq!(IndexType::Property.as_str(), "PropertyIndex");
}

#[test]
fn test_index_lookup_json_serialization() {
    // Arrange
    let plan = QueryPlan {
        root: PlanNode::IndexLookup(IndexLookupPlan {
            label: "Document".to_string(),
            property: "category".to_string(),
            value: "tech".to_string(),
        }),
        estimated_cost_ms: 0.0001,
        index_used: Some(IndexType::Property),
        filter_strategy: FilterStrategy::None,
    };

    // Act
    let json = plan.to_json().expect("JSON serialization failed");

    // Assert
    assert!(json.contains("IndexLookup"));
    assert!(json.contains("Document"));
    assert!(json.contains("category"));
    assert!(json.contains("tech"));
}

// =========================================================================
// EPIC-046 US-004: EXPLAIN MATCH tests (migrated from inline)
// =========================================================================

#[test]
fn test_match_traversal_plan_node() {
    let mt = MatchTraversalPlan {
        strategy: "GraphFirst: Traverse from nodes with labels [Person], max depth 3".to_string(),
        start_labels: vec!["Person".to_string()],
        max_depth: 3,
        relationship_count: 2,
        has_similarity: false,
        similarity_threshold: None,
    };

    let cost = QueryPlan::node_cost(&PlanNode::MatchTraversal(mt.clone()));
    assert!(cost > 0.1);
    assert!(cost < 1.0);
}

#[test]
fn test_render_match_traversal() {
    let mt = PlanNode::MatchTraversal(MatchTraversalPlan {
        strategy: "GraphFirst: max depth 2".to_string(),
        start_labels: vec!["Document".to_string()],
        max_depth: 2,
        relationship_count: 1,
        has_similarity: false,
        similarity_threshold: None,
    });

    let mut output = String::new();
    QueryPlan::render_node(&mt, &mut output, "", true);
    assert!(output.contains("MatchTraversal"));
    assert!(output.contains("GraphFirst"));
    assert!(output.contains("Document"));
    assert!(output.contains("Max Depth: 2"));
}

#[test]
fn test_render_match_traversal_with_similarity() {
    let mt = PlanNode::MatchTraversal(MatchTraversalPlan {
        strategy: "VectorFirst: top-100 candidates".to_string(),
        start_labels: vec![],
        max_depth: 1,
        relationship_count: 0,
        has_similarity: true,
        similarity_threshold: Some(0.85),
    });

    let mut output = String::new();
    QueryPlan::render_node(&mt, &mut output, "", true);
    assert!(output.contains("MatchTraversal"));
    assert!(output.contains("VectorFirst"));
    assert!(output.contains("Similarity Threshold: 0.85"));
}

#[test]
fn test_match_traversal_cost_with_depth() {
    let shallow = MatchTraversalPlan {
        strategy: "GraphFirst".to_string(),
        start_labels: vec![],
        max_depth: 1,
        relationship_count: 1,
        has_similarity: false,
        similarity_threshold: None,
    };

    let deep = MatchTraversalPlan {
        strategy: "GraphFirst".to_string(),
        start_labels: vec![],
        max_depth: 5,
        relationship_count: 5,
        has_similarity: false,
        similarity_threshold: None,
    };

    let shallow_cost = QueryPlan::node_cost(&PlanNode::MatchTraversal(shallow));
    let deep_cost = QueryPlan::node_cost(&PlanNode::MatchTraversal(deep));

    assert!(deep_cost > shallow_cost);
}

#[test]
fn test_explain_output_struct() {
    let plan = QueryPlan {
        root: PlanNode::TableScan(TableScanPlan {
            collection: "test".to_string(),
        }),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
    };

    let output = ExplainOutput {
        plan,
        actual_stats: Some(ActualStats {
            actual_rows: 100,
            actual_time_ms: 0.5,
            loops: 1,
            nodes_visited: 50,
            edges_traversed: 25,
        }),
    };

    assert_eq!(output.actual_stats.as_ref().unwrap().actual_rows, 100);
    assert!(output.actual_stats.as_ref().unwrap().actual_time_ms < 1.0);
}

#[test]
fn test_filter_strategy_default() {
    let strategy = FilterStrategy::default();
    assert_eq!(strategy, FilterStrategy::None);
}

#[test]
fn test_filter_strategy_as_str() {
    assert_eq!(FilterStrategy::None.as_str(), "none");
    assert_eq!(
        FilterStrategy::PreFilter.as_str(),
        "pre-filtering (high selectivity)"
    );
    assert_eq!(
        FilterStrategy::PostFilter.as_str(),
        "post-filtering (low selectivity)"
    );
}

#[test]
fn test_node_cost_calculations() {
    let vs_plan = VectorSearchPlan {
        collection: "test".to_string(),
        ef_search: 100,
        candidates: 50,
    };
    let vs_cost = QueryPlan::node_cost(&PlanNode::VectorSearch(vs_plan));
    assert!((vs_cost - 0.05).abs() < 1e-5);

    let limit_cost = QueryPlan::node_cost(&PlanNode::Limit(LimitPlan { count: 10 }));
    assert!((limit_cost - 0.001).abs() < 1e-5);

    let ts_cost = QueryPlan::node_cost(&PlanNode::TableScan(TableScanPlan {
        collection: "test".to_string(),
    }));
    assert!((ts_cost - 1.0).abs() < 1e-5);

    let il_cost = QueryPlan::node_cost(&PlanNode::IndexLookup(IndexLookupPlan {
        label: "Person".to_string(),
        property: "id".to_string(),
        value: "123".to_string(),
    }));
    assert!((il_cost - 0.0001).abs() < 1e-6);
}

#[test]
fn test_estimate_selectivity() {
    let empty: Vec<String> = vec![];
    let one = vec!["a = ?".to_string()];
    let two = vec!["a = ?".to_string(), "b = ?".to_string()];

    let s0 = QueryPlan::estimate_selectivity(&empty);
    let s1 = QueryPlan::estimate_selectivity(&one);
    let s2 = QueryPlan::estimate_selectivity(&two);

    assert!(s0 > s1);
    assert!(s1 > s2);
}
