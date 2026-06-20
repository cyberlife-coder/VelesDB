//! Tests for `explain` module

use std::collections::HashSet;

use super::ast::{
    CompareOp, Comparison, Condition, InCondition, SelectColumns, SelectStatement, Value,
    VectorExpr, VectorSearch as VsCondition,
};
use super::explain::*;

#[test]
fn test_plan_from_simple_select() {
    // Arrange
    let stmt = SelectStatement {
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
        from_alias: vec![],
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
        from_alias: vec![],
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
    assert_eq!(plan.filter_strategy, FilterStrategy::PostFilter);
}

#[test]
fn test_plan_to_tree_format() {
    // Arrange
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "documents".to_string(),
        from_alias: vec![],
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
        from_alias: vec![],
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
        from_alias: vec![],
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
        from_alias: vec![],
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
        from_alias: vec![],
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
        with_options: Vec::new(),
        let_bindings: Vec::new(),
        fusion_info: None,
        cache_hit: None,
        plan_reuse_count: None,
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
        with_options: Vec::new(),
        let_bindings: Vec::new(),
        fusion_info: None,
        cache_hit: None,
        plan_reuse_count: None,
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
// Cache hit / plan reuse count in to_tree() output
// =========================================================================

#[test]
fn test_plan_to_tree_cache_hit_present() {
    let plan = QueryPlan {
        root: PlanNode::TableScan(TableScanPlan {
            collection: "docs".to_string(),
        }),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
        with_options: Vec::new(),
        let_bindings: Vec::new(),
        fusion_info: None,
        cache_hit: Some(true),
        plan_reuse_count: Some(42),
    };

    let tree = plan.to_tree();
    assert!(
        tree.contains("Cache hit: true"),
        "tree should contain cache hit line"
    );
    assert!(
        tree.contains("Plan reuse count: 42"),
        "tree should contain plan reuse count line"
    );
}

#[test]
fn test_plan_to_tree_cache_hit_absent() {
    let plan = QueryPlan {
        root: PlanNode::TableScan(TableScanPlan {
            collection: "docs".to_string(),
        }),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
        with_options: Vec::new(),
        let_bindings: Vec::new(),
        fusion_info: None,
        cache_hit: None,
        plan_reuse_count: None,
    };

    let tree = plan.to_tree();
    assert!(
        !tree.contains("Cache hit"),
        "tree should NOT contain cache hit when None"
    );
    assert!(
        !tree.contains("Plan reuse count"),
        "tree should NOT contain plan reuse count when None"
    );
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

    let limit_cost = QueryPlan::node_cost(&PlanNode::Limit(LimitPlan {
        count: 10,
        is_default: false,
    }));
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

// =========================================================================
// Issue #471: ef_search from WITH clause
// =========================================================================

#[test]
fn test_ef_search_reads_with_clause() {
    use super::ast::{WithClause, WithValue};

    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "embeddings".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("q".to_string()),
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: Some(WithClause::new().with_option("ef_search", WithValue::Integer(512))),
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();
    assert!(tree.contains("ef_search: 512"), "tree: {tree}");
}

#[test]
fn test_ef_search_defaults_to_100_without_with() {
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "embeddings".to_string(),
        from_alias: vec![],
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

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();
    assert!(tree.contains("ef_search: 100"), "tree: {tree}");
}

// =========================================================================
// Issue #471: WITH options in EXPLAIN output
// =========================================================================

#[test]
fn test_with_options_displayed_in_tree() {
    use super::ast::{WithClause, WithValue};

    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("v".to_string()),
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: Some(
            WithClause::new()
                .with_option("mode", WithValue::Identifier("accurate".to_string()))
                .with_option("ef_search", WithValue::Integer(512))
                .with_option("rerank", WithValue::Boolean(true))
                .with_option("timeout_ms", WithValue::Integer(5000)),
        ),
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();

    assert!(tree.contains("WITH options:"), "tree: {tree}");
    assert!(tree.contains("mode = accurate"), "tree: {tree}");
    assert!(tree.contains("ef_search = 512"), "tree: {tree}");
    assert!(tree.contains("rerank = true"), "tree: {tree}");
    assert!(tree.contains("timeout_ms = 5000"), "tree: {tree}");
}

#[test]
fn test_no_with_options_when_clause_absent() {
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
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
    };

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();
    assert!(!tree.contains("WITH options:"), "tree: {tree}");
}

// =========================================================================
// Issue #471: LET bindings in EXPLAIN output
// =========================================================================

#[test]
fn test_let_bindings_displayed_via_from_query() {
    use super::ast::{ArithmeticExpr, LetBinding, Query};

    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("v".to_string()),
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let query = Query {
        let_bindings: vec![LetBinding {
            name: "hybrid".to_string(),
            expr: ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::BinaryOp {
                    left: Box::new(ArithmeticExpr::Literal(0.7)),
                    op: super::ast::ArithmeticOp::Mul,
                    right: Box::new(ArithmeticExpr::Variable("vector_score".to_string())),
                }),
                op: super::ast::ArithmeticOp::Add,
                right: Box::new(ArithmeticExpr::BinaryOp {
                    left: Box::new(ArithmeticExpr::Literal(0.3)),
                    op: super::ast::ArithmeticOp::Mul,
                    right: Box::new(ArithmeticExpr::Variable("bm25_score".to_string())),
                }),
            },
        }],
        select: stmt,
        compound: None,
        match_clause: None,
        dml: None,
        train: None,
        ddl: None,
        introspection: None,
        admin: None,
    };

    let plan = QueryPlan::from_query(&query);
    let tree = plan.to_tree();

    assert!(tree.contains("LET bindings:"), "tree: {tree}");
    assert!(tree.contains("hybrid = "), "tree: {tree}");
    assert!(tree.contains("vector_score"), "tree: {tree}");
    assert!(tree.contains("bm25_score"), "tree: {tree}");
}

#[test]
fn test_no_let_bindings_when_empty() {
    use super::ast::Query;

    let query = Query::new_select(SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: None,
        order_by: None,
        limit: None,
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    });

    let plan = QueryPlan::from_query(&query);
    let tree = plan.to_tree();
    assert!(!tree.contains("LET bindings:"), "tree: {tree}");
}

// =========================================================================
// Issue #471: FUSION info in EXPLAIN output
// =========================================================================

#[test]
fn test_fusion_rrf_displayed_in_tree() {
    use super::ast::FusionClause;

    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("v".to_string()),
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: Some(FusionClause::default()),
    };

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();

    assert!(tree.contains("FUSION:"), "tree: {tree}");
    assert!(tree.contains("Strategy: RRF"), "tree: {tree}");
    assert!(tree.contains("k: 60"), "tree: {tree}");
}

#[test]
fn test_fusion_weighted_with_weights() {
    use super::ast::{FusionClause, FusionStrategyType};

    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("v".to_string()),
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: Some(FusionClause {
            strategy: FusionStrategyType::Weighted,
            k: None,
            vector_weight: Some(0.7),
            graph_weight: Some(0.3),
            dense_weight: None,
            sparse_weight: None,
        }),
    };

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();

    assert!(tree.contains("FUSION:"), "tree: {tree}");
    assert!(tree.contains("Strategy: Weighted"), "tree: {tree}");
    assert!(
        tree.contains("Weights: vector=0.7, graph=0.3"),
        "tree: {tree}"
    );
    assert!(
        !tree.contains("k:"),
        "tree should not show k when None: {tree}"
    );
}

#[test]
fn test_no_fusion_when_absent() {
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: None,
        order_by: None,
        limit: None,
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();
    assert!(!tree.contains("FUSION:"), "tree: {tree}");
}

// =========================================================================
// Issue #471: JSON serialization of new fields
// =========================================================================

#[test]
fn test_with_options_serialized_in_json() {
    use super::ast::{WithClause, WithValue};

    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "test".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::VectorSearch(VsCondition {
            vector: VectorExpr::Parameter("v".to_string()),
        })),
        order_by: None,
        limit: Some(5),
        offset: None,
        with_clause: Some(WithClause::new().with_option("ef_search", WithValue::Integer(256))),
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let plan = QueryPlan::from_select(&stmt);
    let json = plan.to_json().expect("JSON should serialize");

    assert!(json.contains("\"with_options\""), "json: {json}");
    assert!(json.contains("ef_search"), "json: {json}");
    assert!(json.contains("256"), "json: {json}");
}

#[test]
fn test_empty_with_options_skipped_in_json() {
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "test".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: None,
        order_by: None,
        limit: None,
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let plan = QueryPlan::from_select(&stmt);
    let json = plan.to_json().expect("JSON should serialize");

    assert!(
        !json.contains("\"with_options\""),
        "empty with_options should be skipped: {json}"
    );
    assert!(
        !json.contains("\"let_bindings\""),
        "empty let_bindings should be skipped: {json}"
    );
    assert!(
        !json.contains("\"fusion_info\""),
        "None fusion_info should be skipped: {json}"
    );
}

// ── EXPLAIN IN plan visibility tests (Task 7) ──────────────────────────

#[test]
fn test_explain_in_indexed_shows_prefilter() {
    // Arrange: IN on an indexed field should produce IndexLookup + PreFilter
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::In(InCondition {
            column: "category".to_string(),
            values: vec![
                Value::String("tech".to_string()),
                Value::String("science".to_string()),
            ],
            negated: false,
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let mut indexed_fields = HashSet::new();
    indexed_fields.insert("category".to_string());
    let plan = QueryPlan::from_select_with_indexed_fields(&stmt, &indexed_fields);

    // Assert: should use PropertyIndex and show IndexLookup node
    assert_eq!(plan.index_used, Some(IndexType::Property));
    assert!(
        matches!(plan.root, PlanNode::Sequence(ref nodes) if nodes.iter().any(|n| matches!(n, PlanNode::IndexLookup(_)))),
        "Expected IndexLookup node in plan, got: {:?}",
        plan.root
    );
}

#[test]
fn test_explain_in_unindexed_shows_postfilter() {
    // Arrange: IN on a non-indexed field should produce TableScan + PostFilter
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::In(InCondition {
            column: "category".to_string(),
            values: vec![
                Value::String("tech".to_string()),
                Value::String("science".to_string()),
            ],
            negated: false,
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    // No indexed fields → should fall back to TableScan
    let plan = QueryPlan::from_select_with_indexed_fields(&stmt, &HashSet::new());

    assert_eq!(plan.index_used, None);
    assert_eq!(plan.filter_strategy, FilterStrategy::PostFilter);
    assert!(
        matches!(plan.root, PlanNode::Sequence(ref nodes) if nodes.iter().any(|n| matches!(n, PlanNode::TableScan(_)))),
        "Expected TableScan node in plan, got: {:?}",
        plan.root
    );
}

#[test]
fn test_explain_not_in_indexed_shows_prefilter() {
    // Arrange: NOT IN on an indexed field should also produce IndexLookup
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: Some(Condition::In(InCondition {
            column: "category".to_string(),
            values: vec![
                Value::String("draft".to_string()),
                Value::String("deleted".to_string()),
            ],
            negated: true,
        })),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let mut indexed_fields = HashSet::new();
    indexed_fields.insert("category".to_string());
    let plan = QueryPlan::from_select_with_indexed_fields(&stmt, &indexed_fields);

    assert_eq!(plan.index_used, Some(IndexType::Property));
    assert!(
        matches!(plan.root, PlanNode::Sequence(ref nodes) if nodes.iter().any(|n| matches!(n, PlanNode::IndexLookup(_)))),
        "Expected IndexLookup node for NOT IN on indexed field, got: {:?}",
        plan.root
    );
}

// --- CBO feedback calibration (issue #469 Phase 2) ---

#[test]
fn test_explain_output_feedback_calibration_absent_by_default() {
    let output = ExplainOutput::plan_only(QueryPlan {
        root: PlanNode::TableScan(TableScanPlan {
            collection: "c".to_string(),
        }),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
        with_options: vec![],
        let_bindings: vec![],
        fusion_info: None,
        cache_hit: None,
        plan_reuse_count: None,
    });
    assert!(output.feedback_calibration.is_none());
}

#[test]
fn test_explain_output_with_feedback_calibration_roundtrip() {
    let output = ExplainOutput::plan_only(QueryPlan {
        root: PlanNode::TableScan(TableScanPlan {
            collection: "c".to_string(),
        }),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
        with_options: vec![],
        let_bindings: vec![],
        fusion_info: None,
        cache_hit: None,
        plan_reuse_count: None,
    })
    .with_feedback_calibration(0.07, 42);

    let fb = output
        .feedback_calibration
        .as_ref()
        .expect("should be Some");
    assert!((fb.ms_per_cost_unit - 0.07).abs() < f64::EPSILON);
    assert_eq!(fb.sample_count, 42);
}

#[test]
fn test_explain_output_feedback_calibration_serde_roundtrip() {
    let output = ExplainOutput::plan_only(QueryPlan {
        root: PlanNode::TableScan(TableScanPlan {
            collection: "c".to_string(),
        }),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
        with_options: vec![],
        let_bindings: vec![],
        fusion_info: None,
        cache_hit: None,
        plan_reuse_count: None,
    })
    .with_feedback_calibration(0.12, 100);

    let json = serde_json::to_string(&output).expect("serialize");
    assert!(json.contains("feedback_calibration"));
    assert!(json.contains("ms_per_cost_unit"));

    let decoded: ExplainOutput = serde_json::from_str(&json).expect("deserialize");
    let fb = decoded
        .feedback_calibration
        .expect("should survive roundtrip");
    assert!((fb.ms_per_cost_unit - 0.12).abs() < f64::EPSILON);
    assert_eq!(fb.sample_count, 100);
}

#[test]
fn test_query_cost_estimator_with_feedback_overrides_ms_per_unit() {
    use crate::collection::query_cost::{QueryCostEstimator, QueryParams};

    let params = QueryParams::new(10_000, 100, 10);

    let default_est = QueryCostEstimator::default().estimate(&params);
    let feedback_est = QueryCostEstimator::default()
        .with_feedback(0.05)
        .estimate(&params);

    // 0.05 ms/unit is half of the 0.1 default → latency should be roughly halved.
    assert!(
        feedback_est.estimated_latency_ms < default_est.estimated_latency_ms,
        "feedback-calibrated latency ({}) should be lower than default ({})",
        feedback_est.estimated_latency_ms,
        default_est.estimated_latency_ms
    );
    assert!(
        (default_est.estimated_latency_ms / feedback_est.estimated_latency_ms - 2.0).abs() < 0.01,
        "ratio should be 2.0 (0.1/0.05)"
    );
}

// =========================================================================
// Implicit default LIMIT contract: EXPLAIN must be honest about the
// engine-side `DEFAULT_SELECT_LIMIT` applied to SELECT without LIMIT.
// =========================================================================

#[test]
fn test_select_without_limit_exposes_default_limit_node() {
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
        joins: vec![],
        where_clause: None,
        order_by: None,
        limit: None,
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();
    assert!(
        tree.contains("Limit: 10 (default)"),
        "implicit default LIMIT must surface in the plan: {tree}"
    );

    let json = plan.to_json().expect("JSON should serialize");
    assert!(
        json.contains("\"is_default\":true") || json.contains("\"is_default\": true"),
        "default marker must serialize: {json}"
    );
}

#[test]
fn test_select_with_explicit_limit_has_no_default_marker() {
    let stmt = SelectStatement {
        distinct: crate::velesql::DistinctMode::None,
        columns: SelectColumns::All,
        from: "docs".to_string(),
        from_alias: vec![],
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

    let plan = QueryPlan::from_select(&stmt);
    let tree = plan.to_tree();
    assert!(tree.contains("Limit: 5"), "tree: {tree}");
    assert!(
        !tree.contains("(default)"),
        "explicit LIMIT must not be flagged as default: {tree}"
    );
}

#[test]
fn test_match_query_plan_has_no_implicit_limit() {
    let query =
        crate::velesql::Parser::parse("MATCH (n:Item) RETURN n").expect("parse MATCH query");
    let plan = QueryPlan::from_query(&query);
    let tree = plan.to_tree();
    assert!(
        !tree.contains("Limit:"),
        "MATCH without LIMIT has no implicit limit: {tree}"
    );
}

// ---------------------------------------------------------------------------
// to_plan_steps(): single-sourced structured EXPLAIN steps (C11)
// ---------------------------------------------------------------------------

fn plan_steps_for(query: &str) -> Vec<PlanStep> {
    let parsed = crate::velesql::Parser::parse(query).expect("parse query");
    QueryPlan::from_query(&parsed).to_plan_steps()
}

#[test]
fn test_plan_steps_scan_maps_to_fullscan_wire_string() {
    let steps = plan_steps_for("SELECT * FROM docs");
    let first = steps.first().expect("at least one step");
    assert_eq!(first.operation, PlanStepKind::TableScan);
    // REST vocabulary is preserved: TableScan kind renders as "FullScan".
    assert_eq!(first.rest_operation(), "FullScan");
}

#[test]
fn test_plan_steps_default_limit_step() {
    let steps = plan_steps_for("SELECT * FROM docs");
    let limit = steps
        .iter()
        .find(|s| s.operation == PlanStepKind::Limit)
        .expect("a Limit step");
    assert_eq!(limit.rest_operation(), "Limit");
    assert_eq!(limit.estimated_rows, Some(10));
    assert!(
        limit.description.contains("LIMIT 10 (default)"),
        "default limit description: {}",
        limit.description
    );
}

#[test]
fn test_plan_steps_offset_is_folded_into_limit() {
    let steps = plan_steps_for("SELECT * FROM docs LIMIT 5 OFFSET 20");
    assert!(
        steps.iter().all(|s| s.operation != PlanStepKind::Offset),
        "OFFSET must be folded into the Limit step, not emitted standalone"
    );
    let limit = steps
        .iter()
        .find(|s| s.operation == PlanStepKind::Limit)
        .expect("a Limit step");
    assert_eq!(limit.estimated_rows, Some(5));
    assert!(
        limit.description.contains("LIMIT 5") && limit.description.contains("OFFSET 20"),
        "limit step folds offset: {}",
        limit.description
    );
}

#[test]
fn test_plan_steps_join_preserves_typed_wire_string() {
    let steps = plan_steps_for("SELECT * FROM orders JOIN customers ON orders.cid = customers.id");
    let join = steps
        .iter()
        .find(|s| s.operation == PlanStepKind::Join)
        .expect("a Join step");
    // Default JOIN is INNER; wire string stays "{Type}Join" -> "InnerJoin".
    assert_eq!(join.rest_operation(), "InnerJoin");
}

#[test]
fn test_plan_steps_group_aggregate_sort_pipeline_order() {
    let steps = plan_steps_for(
        "SELECT category, COUNT(*) FROM docs GROUP BY category ORDER BY COUNT(*) DESC LIMIT 5",
    );
    let kinds: Vec<PlanStepKind> = steps.iter().map(|s| s.operation).collect();
    for expected in [
        PlanStepKind::GroupBy,
        PlanStepKind::Aggregate,
        PlanStepKind::Sort,
        PlanStepKind::Limit,
    ] {
        assert!(
            kinds.contains(&expected),
            "missing {expected:?} in {kinds:?}"
        );
    }
    // Pipeline order: group -> aggregate -> sort -> limit.
    let pos = |k: PlanStepKind| kinds.iter().position(|x| *x == k).expect("kind present");
    assert!(pos(PlanStepKind::GroupBy) < pos(PlanStepKind::Aggregate));
    assert!(pos(PlanStepKind::Aggregate) < pos(PlanStepKind::Sort));
    assert!(pos(PlanStepKind::Sort) < pos(PlanStepKind::Limit));
}

#[test]
fn test_plan_steps_vector_search_estimated_rows_is_none() {
    // The VectorSearch step no longer echoes the LIMIT; the estimate is on the
    // Limit step, where it is unambiguous.
    let steps = plan_steps_for("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10");
    let vs = steps
        .iter()
        .find(|s| s.operation == PlanStepKind::VectorSearch)
        .expect("a VectorSearch step");
    assert_eq!(vs.estimated_rows, None);
    let limit = steps
        .iter()
        .find(|s| s.operation == PlanStepKind::Limit)
        .expect("a Limit step");
    assert_eq!(limit.estimated_rows, Some(10));
}

#[test]
fn test_plan_steps_standalone_offset_without_limit() {
    // A plan with OFFSET but no LIMIT (compound/MATCH shape) surfaces a
    // dedicated Offset step rather than folding into a nonexistent Limit.
    let plan = QueryPlan {
        root: PlanNode::Sequence(vec![
            PlanNode::TableScan(TableScanPlan {
                collection: "docs".to_string(),
            }),
            PlanNode::Offset(OffsetPlan { count: 7 }),
        ]),
        estimated_cost_ms: 1.0,
        index_used: None,
        filter_strategy: FilterStrategy::None,
        with_options: vec![],
        let_bindings: vec![],
        fusion_info: None,
        cache_hit: None,
        plan_reuse_count: None,
    };
    let steps = plan.to_plan_steps();
    let offset = steps
        .iter()
        .find(|s| s.operation == PlanStepKind::Offset)
        .expect("a standalone Offset step");
    assert_eq!(offset.rest_operation(), "Offset");
    assert!(offset.description.contains('7'), "{}", offset.description);
    assert!(
        steps.iter().all(|s| s.operation != PlanStepKind::Limit),
        "no Limit step should be present"
    );
}

#[test]
fn test_compound_query_plan_has_no_implicit_limit() {
    let query = crate::velesql::Parser::parse("SELECT * FROM a UNION SELECT * FROM b")
        .expect("parse compound query");
    let plan = QueryPlan::from_query(&query);
    let tree = plan.to_tree();
    assert!(
        !tree.contains("Limit:"),
        "compound queries without LIMIT have no implicit limit: {tree}"
    );
}
