//! E2E integration tests for Phase 7: Cross-Store Execution & EXPLAIN Completeness.
//!
//! Exercises all new query paths introduced in Phase 7:
//! - VP-010: `VectorFirst` cross-store execution (NEAR + graph MATCH)
//! - VP-010: Parallel cross-store execution (NEAR + graph MATCH)
//! - VP-010: `QueryPlanner` wiring in `execute_query()`
//! - VP-012: EXPLAIN `FusedSearch` node for `NEAR_FUSED`
//! - VP-010: EXPLAIN `CrossStoreSearch` node for combined V+G

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::collection::graph::GraphEdge;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::velesql::{
    Direction, GraphPattern, MatchClause, NodePattern, QueryPlan, RelationshipPattern,
    ReturnClause, ReturnItem,
};
use velesdb_core::{Database, Point};

/// Setup a collection with vector data AND graph edges for cross-store E2E tests.
fn setup_phase7_collection() -> (tempfile::TempDir, velesdb_core::Collection) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    db.create_collection("articles", 4, DistanceMetric::Cosine)
        .expect("create");
    let col = db.get_collection("articles").expect("get");

    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"title": "Rust systems guide", "category": "tech", "label": "Article"})),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({"title": "Rust async patterns", "category": "tech", "label": "Article"})),
        ),
        Point::new(
            3,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"title": "Python ML tutorial", "category": "tech", "label": "Article"})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"title": "Cooking vegetables", "category": "food", "label": "Recipe"})),
        ),
        Point::new(
            5,
            vec![0.7, 0.7, 0.0, 0.0],
            Some(json!({"title": "Advanced Rust memory", "category": "tech", "label": "Article"})),
        ),
        Point::new(
            6,
            vec![0.0, 0.7, 0.7, 0.0],
            Some(
                json!({"title": "Data science notebooks", "category": "science", "label": "Article"}),
            ),
        ),
    ];
    col.upsert(points).expect("upsert");

    // Graph edges: RELATED_TO between tech articles
    let edges = [
        (100_u64, 1_u64, 2_u64, "RELATED_TO"),
        (101, 1, 5, "RELATED_TO"),
        (102, 2, 5, "RELATED_TO"),
        (103, 3, 6, "RELATED_TO"),
    ];
    for (eid, src, tgt, label) in &edges {
        let edge = GraphEdge::new(*eid, *src, *tgt, label).expect("edge");
        col.add_edge(edge).expect("add_edge");
    }

    (dir, col)
}

/// Build a `RELATED_TO` match clause for testing.
fn build_related_to_match(limit: Option<u64>) -> MatchClause {
    let pattern = GraphPattern {
        name: None,
        nodes: vec![
            NodePattern::new().with_alias("a"),
            NodePattern::new().with_alias("b"),
        ],
        relationships: vec![RelationshipPattern {
            alias: None,
            types: vec!["RELATED_TO".to_string()],
            direction: Direction::Outgoing,
            range: None,
            properties: HashMap::new(),
        }],
    };

    MatchClause {
        patterns: vec![pattern],
        where_clause: None,
        return_clause: ReturnClause {
            items: vec![ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit,
        },
    }
}

// ============================================================================
// E2E: Cross-store VectorFirst execution
// ============================================================================

#[test]
fn e2e_cross_store_vector_first_with_graph() {
    let (_dir, col) = setup_phase7_collection();

    let query_vec = vec![1.0, 0.0, 0.0, 0.0];
    let match_clause = build_related_to_match(Some(10));

    let results = col
        .execute_vector_first_cross_store(&query_vec, &match_clause, &HashMap::new(), 10)
        .expect("VectorFirst E2E");

    // Should return results that participate in RELATED_TO edges
    assert!(!results.is_empty(), "Should return graph-validated results");

    // Results should be sorted by vector score
    for w in results.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "Results should be sorted by score: {} >= {}",
            w[0].score,
            w[1].score
        );
    }

    // Point 4 (food/Recipe) should NOT appear — no RELATED_TO edges
    assert!(
        !results.iter().any(|r| r.point.id == 4),
        "Food recipe should not appear in RELATED_TO results"
    );
}

// ============================================================================
// E2E: Cross-store Parallel execution
// ============================================================================

#[test]
fn e2e_cross_store_parallel_with_graph() {
    let (_dir, col) = setup_phase7_collection();

    let query_vec = vec![1.0, 0.0, 0.0, 0.0];
    let match_clause = build_related_to_match(Some(10));

    let results = col
        .execute_parallel_cross_store(&query_vec, &match_clause, &HashMap::new(), 10)
        .expect("Parallel E2E");

    // Parallel fuses V+G results — should return non-empty
    assert!(!results.is_empty(), "Parallel should return fused results");

    // Limit should be respected
    assert!(results.len() <= 10, "Should respect limit");
}

// ============================================================================
// E2E: Combined V+G via execute_query()
// ============================================================================

#[test]
fn e2e_cross_store_via_execute_query() {
    let (_dir, col) = setup_phase7_collection();

    let near_cond =
        velesdb_core::velesql::Condition::VectorSearch(velesdb_core::velesql::VectorSearch {
            vector: velesdb_core::velesql::VectorExpr::Parameter("v".to_string()),
        });
    let query = velesdb_core::velesql::Query {
        select: velesdb_core::velesql::SelectStatement {
            distinct: velesdb_core::velesql::DistinctMode::None,
            columns: velesdb_core::velesql::SelectColumns::All,
            from: "articles".to_string(),
            from_alias: None,
            joins: Vec::new(),
            where_clause: Some(near_cond),
            order_by: None,
            limit: Some(10),
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
        match_clause: Some(build_related_to_match(Some(10))),
    };

    let mut params = HashMap::new();
    params.insert("v".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = col.execute_query(&query, &params).expect("combined V+G");
    assert!(
        !results.is_empty(),
        "Combined V+G query via execute_query should return results"
    );
}

// ============================================================================
// E2E: No regression on pure NEAR queries
// ============================================================================

#[test]
fn e2e_cross_store_no_regression_pure_near() {
    let (_dir, col) = setup_phase7_collection();

    let near_cond =
        velesdb_core::velesql::Condition::VectorSearch(velesdb_core::velesql::VectorSearch {
            vector: velesdb_core::velesql::VectorExpr::Parameter("v".to_string()),
        });
    let query = velesdb_core::velesql::Query {
        select: velesdb_core::velesql::SelectStatement {
            distinct: velesdb_core::velesql::DistinctMode::None,
            columns: velesdb_core::velesql::SelectColumns::All,
            from: "articles".to_string(),
            from_alias: None,
            joins: Vec::new(),
            where_clause: Some(near_cond),
            order_by: None,
            limit: Some(10),
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
        match_clause: None, // No match_clause → pure NEAR
    };

    let mut params = HashMap::new();
    params.insert("v".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = col
        .execute_query(&query, &params)
        .expect("pure NEAR should still work");
    assert_eq!(results.len(), 6, "Pure NEAR returns all 6 points");
}

// ============================================================================
// E2E: No regression on pure MATCH queries
// ============================================================================

#[test]
fn e2e_cross_store_no_regression_pure_match() {
    let (_dir, col) = setup_phase7_collection();

    let match_clause = build_related_to_match(Some(10));

    let results = col
        .execute_match(&match_clause, &HashMap::new())
        .expect("pure MATCH should still work");
    assert!(
        !results.is_empty(),
        "Pure MATCH should return graph traversal results"
    );
}

// ============================================================================
// E2E: EXPLAIN for NEAR_FUSED query
// ============================================================================

#[test]
fn e2e_explain_near_fused_via_parser() {
    // Parse a NEAR_FUSED query via the VelesQL parser
    let query_str =
        "SELECT * FROM docs WHERE vector NEAR_FUSED [$v1, $v2] USING FUSION 'rrf' LIMIT 10";
    let parsed = velesdb_core::velesql::Parser::parse(query_str);

    // If parser supports NEAR_FUSED, verify EXPLAIN output
    if let Ok(query) = parsed {
        let plan = QueryPlan::from_select(&query.select);
        let tree = plan.to_tree();

        // Should show FusedSearch node, not VectorSearch
        assert!(
            tree.contains("FusedSearch"),
            "EXPLAIN of NEAR_FUSED should show FusedSearch node, got: {tree}"
        );
    }
    // If parser doesn't support NEAR_FUSED syntax yet, test with manual construction
    else {
        let fused = velesdb_core::velesql::Condition::VectorFusedSearch(
            velesdb_core::velesql::VectorFusedSearch {
                vectors: vec![
                    velesdb_core::velesql::VectorExpr::Parameter("v1".to_string()),
                    velesdb_core::velesql::VectorExpr::Parameter("v2".to_string()),
                ],
                fusion: velesdb_core::velesql::FusionConfig::rrf(),
            },
        );
        let stmt = velesdb_core::velesql::SelectStatement {
            distinct: velesdb_core::velesql::DistinctMode::None,
            columns: velesdb_core::velesql::SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
            joins: Vec::new(),
            where_clause: Some(fused),
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
        assert!(
            tree.contains("FusedSearch"),
            "EXPLAIN should show FusedSearch: {tree}"
        );
    }
}

// ============================================================================
// E2E: EXPLAIN for cross-store combined V+G query
// ============================================================================

#[test]
fn e2e_explain_cross_store_combined() {
    let stmt = velesdb_core::velesql::SelectStatement {
        distinct: velesdb_core::velesql::DistinctMode::None,
        columns: velesdb_core::velesql::SelectColumns::All,
        from: "articles".to_string(),
        from_alias: None,
        joins: Vec::new(),
        where_clause: Some(velesdb_core::velesql::Condition::VectorSearch(
            velesdb_core::velesql::VectorSearch {
                vector: velesdb_core::velesql::VectorExpr::Parameter("v".to_string()),
            },
        )),
        order_by: None,
        limit: Some(10),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    };

    let match_clause = build_related_to_match(Some(10));

    let plan = QueryPlan::from_combined(&stmt, &match_clause, false, false);
    let tree = plan.to_tree();

    assert!(
        tree.contains("CrossStoreSearch"),
        "EXPLAIN should show CrossStoreSearch node: {tree}"
    );
    assert!(
        tree.contains("Parallel") || tree.contains("VectorFirst"),
        "Should show strategy: {tree}"
    );
}
