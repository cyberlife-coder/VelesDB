//! Tests for multi-hop MATCH chain traversal (VP-004).
//!
//! Verifies that MATCH patterns with multiple relationships execute as
//! hop-by-hop chains with per-hop relationship type filtering and
//! proper intermediate node bindings.

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;

use crate::collection::graph::GraphEdge;
use crate::velesql::{
    CompareOp, Comparison, Condition, Direction, GraphPattern, MatchClause, NodePattern,
    RelationshipPattern, ReturnClause, ReturnItem, Value,
};
use crate::{Database, DistanceMetric, Point};

/// Creates a test collection with a multi-hop graph structure:
///
/// ```text
/// Person:Alice --[:WORKS_AT]--> Company:Acme --[:LOCATED_IN]--> City:Paris
/// Person:Bob   --[:WORKS_AT]--> Company:Acme --[:LOCATED_IN]--> City:Paris
/// Person:Carol --[:WORKS_AT]--> Company:Beta --[:LOCATED_IN]--> City:London
/// Person:Alice --[:FRIEND]--> Person:Bob
/// ```
fn setup_multi_hop_collection() -> (TempDir, crate::Collection) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");

    db.create_collection("graph_test", 4, DistanceMetric::Cosine)
        .expect("Failed to create collection");
    let collection = db
        .get_collection("graph_test")
        .expect("Failed to get collection");

    // Insert nodes with labels and properties
    let points = vec![
        // Persons
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Person"],
                "name": "Alice",
                "age": 30
            })),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Person"],
                "name": "Bob",
                "age": 25
            })),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "_labels": ["Person"],
                "name": "Carol",
                "age": 35
            })),
        ),
        // Companies
        Point::new(
            10,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({
                "_labels": ["Company"],
                "name": "Acme",
                "industry": "tech"
            })),
        ),
        Point::new(
            11,
            vec![0.0, 0.5, 0.5, 0.0],
            Some(json!({
                "_labels": ["Company"],
                "name": "Beta",
                "industry": "finance"
            })),
        ),
        // Cities
        Point::new(
            20,
            vec![0.0, 0.0, 0.5, 0.5],
            Some(json!({
                "_labels": ["City"],
                "name": "Paris",
                "country": "France"
            })),
        ),
        Point::new(
            21,
            vec![0.5, 0.0, 0.0, 0.5],
            Some(json!({
                "_labels": ["City"],
                "name": "London",
                "country": "UK"
            })),
        ),
    ];

    collection.upsert(points).expect("Failed to upsert points");

    // Add edges
    {
        let mut edge_store = collection.edge_store.write();

        // Person -[:WORKS_AT]-> Company
        edge_store
            .add_edge(GraphEdge::new(100, 1, 10, "WORKS_AT").unwrap())
            .expect("add edge");
        edge_store
            .add_edge(GraphEdge::new(101, 2, 10, "WORKS_AT").unwrap())
            .expect("add edge");
        edge_store
            .add_edge(GraphEdge::new(102, 3, 11, "WORKS_AT").unwrap())
            .expect("add edge");

        // Company -[:LOCATED_IN]-> City
        edge_store
            .add_edge(GraphEdge::new(200, 10, 20, "LOCATED_IN").unwrap())
            .expect("add edge");
        edge_store
            .add_edge(GraphEdge::new(201, 11, 21, "LOCATED_IN").unwrap())
            .expect("add edge");

        // Person -[:FRIEND]-> Person
        edge_store
            .add_edge(GraphEdge::new(300, 1, 2, "FRIEND").unwrap())
            .expect("add edge");
    }

    (temp_dir, collection)
}

/// Helper: build a simple RETURN clause.
fn return_clause_with_limit(limit: u64) -> ReturnClause {
    ReturnClause {
        items: vec![ReturnItem {
            expression: "*".to_string(),
            alias: None,
        }],
        order_by: None,
        limit: Some(limit),
    }
}

/// Helper: build a RETURN clause with property projections.
fn return_clause_with_projections(exprs: &[&str], limit: u64) -> ReturnClause {
    ReturnClause {
        items: exprs
            .iter()
            .map(|e| ReturnItem {
                expression: e.to_string(),
                alias: None,
            })
            .collect(),
        order_by: None,
        limit: Some(limit),
    }
}

// ============================================================================
// Test 1: Two-hop chain with different relationship types
// ============================================================================

#[test]
fn test_two_hop_chain_with_different_rel_types() {
    let (_tmp, collection) = setup_multi_hop_collection();

    // MATCH (p:Person)-[:WORKS_AT]->(c:Company)-[:LOCATED_IN]->(city:City)
    // RETURN p.name, c.name, city.name
    let match_clause = MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![
                NodePattern::new().with_alias("p").with_label("Person"),
                NodePattern::new().with_alias("c").with_label("Company"),
                NodePattern::new().with_alias("city").with_label("City"),
            ],
            relationships: vec![
                {
                    let mut rel = RelationshipPattern::new(Direction::Outgoing);
                    rel.types = vec!["WORKS_AT".to_string()];
                    rel
                },
                {
                    let mut rel = RelationshipPattern::new(Direction::Outgoing);
                    rel.types = vec!["LOCATED_IN".to_string()];
                    rel
                },
            ],
        }],
        where_clause: None,
        return_clause: return_clause_with_projections(&["p.name", "c.name", "city.name"], 100),
    };

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    // Should get 3 results: Alice→Acme→Paris, Bob→Acme→Paris, Carol→Beta→London
    assert_eq!(
        results.len(),
        3,
        "Expected 3 two-hop results, got {}. Results: {:?}",
        results.len(),
        results
            .iter()
            .map(|r| (&r.bindings, r.node_id))
            .collect::<Vec<_>>()
    );

    // Each result should reach a City node (IDs 20 or 21)
    let city_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(
        city_ids.iter().all(|&id| id == 20 || id == 21),
        "All final nodes should be Cities, got: {:?}",
        city_ids
    );

    // Paris (20) should appear twice (Alice→Acme→Paris, Bob→Acme→Paris)
    let paris_count = city_ids.iter().filter(|&&id| id == 20).count();
    assert_eq!(paris_count, 2, "Paris should be reached twice");

    // London (21) should appear once (Carol→Beta→London)
    let london_count = city_ids.iter().filter(|&&id| id == 21).count();
    assert_eq!(london_count, 1, "London should be reached once");
}

// ============================================================================
// Test 2: Intermediate bindings are populated
// ============================================================================

#[test]
fn test_two_hop_intermediate_bindings() {
    let (_tmp, collection) = setup_multi_hop_collection();

    // MATCH (p:Person)-[:WORKS_AT]->(c:Company)-[:LOCATED_IN]->(city:City)
    let match_clause = MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![
                NodePattern::new().with_alias("p").with_label("Person"),
                NodePattern::new().with_alias("c").with_label("Company"),
                NodePattern::new().with_alias("city").with_label("City"),
            ],
            relationships: vec![
                {
                    let mut rel = RelationshipPattern::new(Direction::Outgoing);
                    rel.types = vec!["WORKS_AT".to_string()];
                    rel
                },
                {
                    let mut rel = RelationshipPattern::new(Direction::Outgoing);
                    rel.types = vec!["LOCATED_IN".to_string()];
                    rel
                },
            ],
        }],
        where_clause: None,
        return_clause: return_clause_with_limit(100),
    };

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    assert!(!results.is_empty(), "Should have results");

    // Every result should have bindings for all three aliases: p, c, city
    for result in &results {
        assert!(
            result.bindings.contains_key("p"),
            "Missing 'p' binding in result: {:?}",
            result.bindings
        );
        assert!(
            result.bindings.contains_key("c"),
            "Missing 'c' binding in result: {:?}",
            result.bindings
        );
        assert!(
            result.bindings.contains_key("city"),
            "Missing 'city' binding in result: {:?}",
            result.bindings
        );

        // 'p' should be a Person (1, 2, or 3)
        let p_id = result.bindings["p"];
        assert!(
            p_id == 1 || p_id == 2 || p_id == 3,
            "'p' should be a Person node, got {}",
            p_id
        );

        // 'c' should be a Company (10 or 11)
        let c_id = result.bindings["c"];
        assert!(
            c_id == 10 || c_id == 11,
            "'c' should be a Company node, got {}",
            c_id
        );

        // 'city' should be a City (20 or 21)
        let city_id = result.bindings["city"];
        assert!(
            city_id == 20 || city_id == 21,
            "'city' should be a City node, got {}",
            city_id
        );
    }
}

// ============================================================================
// Test 3: WHERE on intermediate node
// ============================================================================

#[test]
fn test_two_hop_where_on_intermediate_node() {
    let (_tmp, collection) = setup_multi_hop_collection();

    // MATCH (p:Person)-[:WORKS_AT]->(c:Company)-[:LOCATED_IN]->(city:City)
    // WHERE c.name = 'Acme'
    let match_clause = MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![
                NodePattern::new().with_alias("p").with_label("Person"),
                NodePattern::new().with_alias("c").with_label("Company"),
                NodePattern::new().with_alias("city").with_label("City"),
            ],
            relationships: vec![
                {
                    let mut rel = RelationshipPattern::new(Direction::Outgoing);
                    rel.types = vec!["WORKS_AT".to_string()];
                    rel
                },
                {
                    let mut rel = RelationshipPattern::new(Direction::Outgoing);
                    rel.types = vec!["LOCATED_IN".to_string()];
                    rel
                },
            ],
        }],
        where_clause: Some(Condition::Comparison(Comparison {
            column: "c.name".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("Acme".to_string()),
        })),
        return_clause: return_clause_with_limit(100),
    };

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    // Only Alice→Acme→Paris and Bob→Acme→Paris should match (Carol→Beta filtered out)
    assert_eq!(
        results.len(),
        2,
        "Expected 2 results (Acme only), got {}. Bindings: {:?}",
        results.len(),
        results.iter().map(|r| &r.bindings).collect::<Vec<_>>()
    );

    // All 'c' bindings should point to Acme (10)
    for result in &results {
        assert_eq!(
            result.bindings.get("c"),
            Some(&10),
            "Company should be Acme (10)"
        );
    }

    // All results should reach Paris (20)
    for result in &results {
        assert_eq!(result.node_id, 20, "City should be Paris (20)");
    }
}

// ============================================================================
// Test 4: Single-hop regression test
// ============================================================================

#[test]
fn test_single_hop_still_works() {
    let (_tmp, collection) = setup_multi_hop_collection();

    // MATCH (p:Person)-[:FRIEND]->(f:Person) RETURN p.name, f.name
    let match_clause = MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![
                NodePattern::new().with_alias("p").with_label("Person"),
                NodePattern::new().with_alias("f").with_label("Person"),
            ],
            relationships: vec![{
                let mut rel = RelationshipPattern::new(Direction::Outgoing);
                rel.types = vec!["FRIEND".to_string()];
                rel
            }],
        }],
        where_clause: None,
        return_clause: return_clause_with_projections(&["p.name", "f.name"], 100),
    };

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    // Alice --[:FRIEND]--> Bob
    assert_eq!(
        results.len(),
        1,
        "Expected 1 FRIEND result, got {}",
        results.len()
    );

    let result = &results[0];
    assert_eq!(result.bindings.get("p"), Some(&1), "p should be Alice (1)");
    assert_eq!(result.bindings.get("f"), Some(&2), "f should be Bob (2)");
}

// ============================================================================
// Test 5: Variable-length hop
// ============================================================================

#[test]
fn test_variable_length_hop() {
    let (_tmp, collection) = setup_multi_hop_collection();

    // MATCH (p:Person)-[:WORKS_AT|LOCATED_IN*1..2]->(target) RETURN target
    // Should return Company nodes at depth 1 and City nodes at depth 2
    let match_clause = MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![
                NodePattern::new().with_alias("p").with_label("Person"),
                NodePattern::new().with_alias("target"),
            ],
            relationships: vec![{
                let mut rel = RelationshipPattern::new(Direction::Outgoing);
                rel.types = vec!["WORKS_AT".to_string(), "LOCATED_IN".to_string()];
                rel.range = Some((1, 2));
                rel
            }],
        }],
        where_clause: None,
        return_clause: return_clause_with_limit(100),
    };

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    // At depth 1: Alice→Acme, Bob→Acme, Carol→Beta, Alice→Bob(friend won't match - wrong types)
    // At depth 2: Alice→Acme→Paris, Bob→Acme→Paris, Carol→Beta→London
    // Total depth-1 results for WORKS_AT|LOCATED_IN: Acme(from Alice), Acme(from Bob), Beta(from Carol) = 3
    // Total depth-2 results: Paris(from Alice via Acme), Paris(from Bob via Acme), London(from Carol via Beta) = 3
    // Total: 6
    assert!(
        results.len() >= 3,
        "Expected at least 3 results from variable-length traversal, got {}",
        results.len()
    );

    // Should include both Company and City nodes
    let result_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    let has_company = result_ids.iter().any(|&id| id == 10 || id == 11);
    let has_city = result_ids.iter().any(|&id| id == 20 || id == 21);
    assert!(has_company, "Should reach Company nodes at depth 1");
    assert!(has_city, "Should reach City nodes at depth 2");
}
