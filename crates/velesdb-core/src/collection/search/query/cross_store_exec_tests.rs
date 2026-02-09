//! TDD tests for cross-store execution (VP-010, Plan 07-01).
//!
//! Tests VectorFirst, Parallel, and GraphFirst strategies for combined
//! NEAR + graph MATCH queries.

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;

use crate::collection::graph::GraphEdge;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::velesql::{
    Direction, GraphPattern, MatchClause, NodePattern, RelationshipPattern, ReturnClause,
    ReturnItem,
};
use crate::{Database, Point};

/// Dimension for test vectors.
const DIM: usize = 4;

/// Setup a collection with vector data AND graph edges for cross-store testing.
///
/// Creates:
/// - 6 points with vectors and payloads (category: tech/food/science)
/// - Graph edges: RELATED_TO between tech documents
fn setup_cross_store_collection() -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    db.create_collection("docs", DIM, DistanceMetric::Cosine)
        .expect("create");
    let col = db.get_collection("docs").expect("get");

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

    // Graph edges: tech articles are RELATED_TO each other
    // 1 -> 2 (Rust systems -> Rust async)
    // 1 -> 5 (Rust systems -> Advanced Rust memory)
    // 2 -> 5 (Rust async -> Advanced Rust memory)
    // 3 -> 6 (Python ML -> Data science)
    let edges = [
        (100, 1_u64, 2_u64, "RELATED_TO"),
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

/// Build a simple single-hop MatchClause: (a)-[:RELATED_TO]->(b)
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
// VectorFirst strategy tests
// ============================================================================

#[test]
fn test_vector_first_returns_graph_validated_results() {
    let (_dir, col) = setup_cross_store_collection();

    let query_vec = vec![1.0, 0.0, 0.0, 0.0]; // Closest to point 1, then 2, then 5
    let match_clause = build_related_to_match(Some(10));

    let results = col
        .execute_vector_first_cross_store(&query_vec, &match_clause, &HashMap::new(), 10)
        .expect("VectorFirst should work");

    // Results should only contain points that participate in RELATED_TO edges
    // Points 1, 2, 3, 5, 6 have RELATED_TO edges; point 4 does not
    assert!(!results.is_empty(), "Should return graph-validated results");

    for r in &results {
        // All results should be points that appear in RELATED_TO edges
        assert!(
            [1, 2, 3, 5, 6].contains(&r.point.id),
            "Result id={} should be in a RELATED_TO edge",
            r.point.id
        );
    }
}

#[test]
fn test_vector_first_respects_limit() {
    let (_dir, col) = setup_cross_store_collection();

    let query_vec = vec![1.0, 0.0, 0.0, 0.0];
    let match_clause = build_related_to_match(Some(2));

    let results = col
        .execute_vector_first_cross_store(&query_vec, &match_clause, &HashMap::new(), 2)
        .expect("VectorFirst with limit");

    assert!(results.len() <= 2, "Should respect limit of 2");
}

#[test]
fn test_vector_first_sorted_by_vector_score() {
    let (_dir, col) = setup_cross_store_collection();

    let query_vec = vec![1.0, 0.0, 0.0, 0.0];
    let match_clause = build_related_to_match(Some(10));

    let results = col
        .execute_vector_first_cross_store(&query_vec, &match_clause, &HashMap::new(), 10)
        .expect("VectorFirst sorted");

    // Results should be sorted by descending score (cosine similarity)
    for w in results.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "Results should be sorted by score descending: {} >= {}",
            w[0].score,
            w[1].score
        );
    }
}

// ============================================================================
// Parallel strategy tests
// ============================================================================

#[test]
fn test_parallel_fuses_results() {
    let (_dir, col) = setup_cross_store_collection();

    let query_vec = vec![1.0, 0.0, 0.0, 0.0];
    let match_clause = build_related_to_match(Some(10));

    let results = col
        .execute_parallel_cross_store(&query_vec, &match_clause, &HashMap::new(), 10)
        .expect("Parallel should work");

    assert!(!results.is_empty(), "Parallel should return fused results");
}

#[test]
fn test_parallel_respects_limit() {
    let (_dir, col) = setup_cross_store_collection();

    let query_vec = vec![1.0, 0.0, 0.0, 0.0];
    let match_clause = build_related_to_match(Some(3));

    let results = col
        .execute_parallel_cross_store(&query_vec, &match_clause, &HashMap::new(), 3)
        .expect("Parallel with limit");

    assert!(results.len() <= 3, "Should respect limit of 3");
}

// ============================================================================
// QueryPlanner wiring tests
// ============================================================================

#[test]
fn test_planner_wired_in_execute_query() {
    let (_dir, col) = setup_cross_store_collection();

    // Build a Query with both SELECT WHERE NEAR and match_clause
    let near_cond = crate::velesql::Condition::VectorSearch(crate::velesql::VectorSearch {
        vector: crate::velesql::VectorExpr::Parameter("v".to_string()),
    });
    let query = crate::velesql::Query {
        select: crate::velesql::SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns: crate::velesql::SelectColumns::All,
            from: "docs".to_string(),
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
        "Combined V+G query should return results"
    );
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_cross_store_no_graph_match_unchanged() {
    let (_dir, col) = setup_cross_store_collection();

    // Pure NEAR query (no match_clause) should still work unchanged
    let near_cond = crate::velesql::Condition::VectorSearch(crate::velesql::VectorSearch {
        vector: crate::velesql::VectorExpr::Parameter("v".to_string()),
    });
    let query = crate::velesql::Query {
        select: crate::velesql::SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns: crate::velesql::SelectColumns::All,
            from: "docs".to_string(),
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
        match_clause: None,
    };

    let mut params = HashMap::new();
    params.insert("v".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = col.execute_query(&query, &params).expect("pure NEAR");
    assert!(!results.is_empty(), "Pure NEAR should still work");
    // Pure NEAR returns all 6 points (no graph filtering)
    assert_eq!(results.len(), 6, "All 6 points should be returned");
}

#[test]
fn test_graph_first_delegates_to_existing() {
    let (_dir, col) = setup_cross_store_collection();

    // Test that execute_match_with_similarity (GraphFirst) still works
    let match_clause = build_related_to_match(Some(10));
    let query_vec = vec![1.0, 0.0, 0.0, 0.0];

    let results = col
        .execute_match_with_similarity(&match_clause, &query_vec, 0.0, &HashMap::new())
        .expect("GraphFirst fallback");

    assert!(
        !results.is_empty(),
        "GraphFirst via execute_match_with_similarity should work"
    );
}
