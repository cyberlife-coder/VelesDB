//! Tests for MATCH RETURN aggregation (VP-005).
//!
//! Verifies that RETURN clauses with aggregation functions (COUNT, AVG, SUM, MIN, MAX)
//! compute correct results with implicit grouping per `OpenCypher` standard.

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;

use crate::collection::graph::GraphEdge;
use crate::velesql::{
    Direction, GraphPattern, MatchClause, NodePattern, RelationshipPattern, ReturnClause,
    ReturnItem,
};
use crate::{Database, DistanceMetric, Point};

/// Creates a test collection for healthcare scenario:
///
/// ```text
/// Doctor:DrSmith --[:TREATS]--> Patient:P1 {condition: 'flu', success_rate: 0.95}
/// Doctor:DrSmith --[:TREATS]--> Patient:P2 {condition: 'flu', success_rate: 0.85}
/// Doctor:DrSmith --[:TREATS]--> Patient:P3 {condition: 'cold', success_rate: 0.90}
/// Doctor:DrJones --[:TREATS]--> Patient:P4 {condition: 'flu', success_rate: 0.80}
/// ```
fn setup_healthcare_collection() -> (TempDir, crate::Collection) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");

    db.create_collection("healthcare", 4, DistanceMetric::Cosine)
        .expect("Failed to create collection");
    let collection = db
        .get_collection("healthcare")
        .expect("Failed to get collection");

    let points = vec![
        // Doctors
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Doctor"],
                "name": "DrSmith"
            })),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Doctor"],
                "name": "DrJones"
            })),
        ),
        // Patients
        Point::new(
            10,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({
                "_labels": ["Patient"],
                "condition": "flu",
                "success_rate": 0.95
            })),
        ),
        Point::new(
            11,
            vec![0.0, 0.5, 0.5, 0.0],
            Some(json!({
                "_labels": ["Patient"],
                "condition": "flu",
                "success_rate": 0.85
            })),
        ),
        Point::new(
            12,
            vec![0.0, 0.0, 0.5, 0.5],
            Some(json!({
                "_labels": ["Patient"],
                "condition": "cold",
                "success_rate": 0.90
            })),
        ),
        Point::new(
            13,
            vec![0.5, 0.0, 0.0, 0.5],
            Some(json!({
                "_labels": ["Patient"],
                "condition": "flu",
                "success_rate": 0.80
            })),
        ),
    ];

    collection.upsert(points).expect("Failed to upsert points");

    // Add edges: Doctor -[:TREATS]-> Patient
    {
        let mut edge_store = collection.edge_store.write();
        edge_store
            .add_edge(GraphEdge::new(100, 1, 10, "TREATS").unwrap())
            .expect("add edge");
        edge_store
            .add_edge(GraphEdge::new(101, 1, 11, "TREATS").unwrap())
            .expect("add edge");
        edge_store
            .add_edge(GraphEdge::new(102, 1, 12, "TREATS").unwrap())
            .expect("add edge");
        edge_store
            .add_edge(GraphEdge::new(103, 2, 13, "TREATS").unwrap())
            .expect("add edge");
    }

    (temp_dir, collection)
}

/// Helper: build a MATCH clause for (d:Doctor)-[:TREATS]->(p:Patient) with given RETURN items.
fn healthcare_match(return_items: Vec<ReturnItem>) -> MatchClause {
    MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![
                NodePattern::new().with_alias("d").with_label("Doctor"),
                NodePattern::new().with_alias("p").with_label("Patient"),
            ],
            relationships: vec![{
                let mut rel = RelationshipPattern::new(Direction::Outgoing);
                rel.types = vec!["TREATS".to_string()];
                rel
            }],
        }],
        where_clause: None,
        return_clause: ReturnClause {
            items: return_items,
            order_by: None,
            limit: Some(100),
        },
    }
}

// ============================================================================
// Test 1: COUNT(*)
// ============================================================================

#[test]
fn test_return_count_star() {
    let (_tmp, collection) = setup_healthcare_collection();

    // MATCH (d:Doctor)-[:TREATS]->(p:Patient) RETURN d.name, COUNT(*)
    let match_clause = healthcare_match(vec![
        ReturnItem {
            expression: "d.name".to_string(),
            alias: None,
        },
        ReturnItem {
            expression: "COUNT(*)".to_string(),
            alias: None,
        },
    ]);

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    // Should get 2 groups: DrSmith (3 patients), DrJones (1 patient)
    assert_eq!(results.len(), 2, "Expected 2 groups, got {}", results.len());

    // Find DrSmith and DrJones groups
    let smith = results
        .iter()
        .find(|r| r.projected.get("d.name") == Some(&json!("DrSmith")));
    let jones = results
        .iter()
        .find(|r| r.projected.get("d.name") == Some(&json!("DrJones")));

    assert!(smith.is_some(), "DrSmith group should exist");
    assert!(jones.is_some(), "DrJones group should exist");

    // DrSmith treats 3 patients
    let smith = smith.unwrap();
    assert_eq!(
        smith.projected.get("COUNT(*)"),
        Some(&json!(3)),
        "DrSmith should have COUNT(*)=3, got: {:?}",
        smith.projected
    );

    // DrJones treats 1 patient
    let jones = jones.unwrap();
    assert_eq!(
        jones.projected.get("COUNT(*)"),
        Some(&json!(1)),
        "DrJones should have COUNT(*)=1, got: {:?}",
        jones.projected
    );
}

// ============================================================================
// Test 2: AVG aggregation
// ============================================================================

#[test]
fn test_return_avg_aggregation() {
    let (_tmp, collection) = setup_healthcare_collection();

    // MATCH (d:Doctor)-[:TREATS]->(p:Patient) RETURN d.name, AVG(p.success_rate)
    let match_clause = healthcare_match(vec![
        ReturnItem {
            expression: "d.name".to_string(),
            alias: None,
        },
        ReturnItem {
            expression: "AVG(p.success_rate)".to_string(),
            alias: None,
        },
    ]);

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    assert_eq!(results.len(), 2, "Expected 2 groups, got {}", results.len());

    let smith = results
        .iter()
        .find(|r| r.projected.get("d.name") == Some(&json!("DrSmith")))
        .expect("DrSmith group should exist");

    // DrSmith: (0.95 + 0.85 + 0.90) / 3 = 0.9
    let avg = smith
        .projected
        .get("AVG(p.success_rate)")
        .and_then(serde_json::Value::as_f64)
        .expect("AVG should be a number");
    assert!(
        (avg - 0.9).abs() < 0.01,
        "DrSmith AVG should be ~0.9, got {avg}"
    );

    let jones = results
        .iter()
        .find(|r| r.projected.get("d.name") == Some(&json!("DrJones")))
        .expect("DrJones group should exist");

    // DrJones: 0.80
    let avg = jones
        .projected
        .get("AVG(p.success_rate)")
        .and_then(serde_json::Value::as_f64)
        .expect("AVG should be a number");
    assert!(
        (avg - 0.80).abs() < 0.01,
        "DrJones AVG should be ~0.80, got {avg}"
    );
}

// ============================================================================
// Test 3: Non-aggregated RETURN unchanged (regression test)
// ============================================================================

#[test]
fn test_return_without_aggregation_unchanged() {
    let (_tmp, collection) = setup_healthcare_collection();

    // MATCH (d:Doctor)-[:TREATS]->(p:Patient) RETURN d.name, p.condition
    // No aggregation → should return individual rows, no grouping
    let match_clause = healthcare_match(vec![
        ReturnItem {
            expression: "d.name".to_string(),
            alias: None,
        },
        ReturnItem {
            expression: "p.condition".to_string(),
            alias: None,
        },
    ]);

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    // Should return 4 individual rows (one per edge), NOT grouped
    assert_eq!(
        results.len(),
        4,
        "Without aggregation, should return 4 individual rows, got {}",
        results.len()
    );
}

// ============================================================================
// Test 4: COUNT(*) with no grouping key (global aggregation)
// ============================================================================

#[test]
fn test_return_count_with_no_grouping_key() {
    let (_tmp, collection) = setup_healthcare_collection();

    // MATCH (d:Doctor)-[:TREATS]->(p:Patient) RETURN COUNT(*)
    // No non-aggregated items → single result row with total count
    let match_clause = healthcare_match(vec![ReturnItem {
        expression: "COUNT(*)".to_string(),
        alias: None,
    }]);

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    // Should return exactly 1 result with total count
    assert_eq!(
        results.len(),
        1,
        "Global COUNT(*) should return 1 row, got {}",
        results.len()
    );

    assert_eq!(
        results[0].projected.get("COUNT(*)"),
        Some(&json!(4)),
        "Total COUNT(*) should be 4, got: {:?}",
        results[0].projected
    );
}

// ============================================================================
// Test 5: Aggregation with alias
// ============================================================================

#[test]
fn test_return_aggregation_with_alias() {
    let (_tmp, collection) = setup_healthcare_collection();

    // MATCH (d:Doctor)-[:TREATS]->(p:Patient) RETURN d.name, COUNT(*) AS patient_count
    let match_clause = healthcare_match(vec![
        ReturnItem {
            expression: "d.name".to_string(),
            alias: None,
        },
        ReturnItem {
            expression: "COUNT(*)".to_string(),
            alias: Some("patient_count".to_string()),
        },
    ]);

    let params = HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    assert_eq!(results.len(), 2, "Expected 2 groups");

    let smith = results
        .iter()
        .find(|r| r.projected.get("d.name") == Some(&json!("DrSmith")))
        .expect("DrSmith group should exist");

    // Alias should be used as key, not the expression
    assert!(
        smith.projected.contains_key("patient_count"),
        "Alias 'patient_count' should be used as key, got keys: {:?}",
        smith.projected.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        smith.projected.get("patient_count"),
        Some(&json!(3)),
        "DrSmith patient_count should be 3"
    );
}
