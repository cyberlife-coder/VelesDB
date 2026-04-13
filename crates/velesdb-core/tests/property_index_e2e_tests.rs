#![cfg(feature = "persistence")]
//! E2E tests for graph property index pipeline.
//!
//! Verifies:
//! 1. `PropertyIndex` auto-population on `create_index` + `on_add_node` hooks
//! 2. `RangeIndex` range queries (GT, LT, BETWEEN, GTE) return correct node sets
//! 3. Graph MATCH queries with WHERE property filters return correct results
//! 4. Property index survives node update (old value removed, new value indexed)
//! 5. `LabelIndex` auto-population on `upsert_node_payload` verified via MATCH
//! 6. Persistence round-trip for both `PropertyIndex` and `RangeIndex`
//! 7. Negative tests: un-indexed lookups, non-existent labels, edge cases
//!
//! Tests exercise the integration between `PropertyIndex`, `RangeIndex`,
//! `GraphCollection`, `LabelIndex`, and the VelesQL MATCH pipeline.

// Reason: test-only casts (u64 literals, loop counters) are safe and bounded.
#![allow(clippy::cast_possible_truncation)]

use std::collections::{HashMap, HashSet};

use tempfile::TempDir;
use velesdb_core::collection::graph::{GraphEdge, GraphSchema, PropertyIndex, RangeIndex};
use velesdb_core::{Database, GraphCollection};

// =========================================================================
// Helpers
// =========================================================================

/// Creates a fresh database with a schemaless graph collection named `"kg"`.
fn setup_graph_db() -> (TempDir, Database, GraphCollection) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_graph_collection("kg", GraphSchema::schemaless())
        .expect("test: create graph collection");
    let gc = db
        .get_graph_collection("kg")
        .expect("test: get graph collection");
    (dir, db, gc)
}

// =========================================================================
// A. PropertyIndex: auto-population on node insert
// =========================================================================

/// GIVEN: A `PropertyIndex` with indexes created for ("Person", "age") and
///        ("Person", "city")
/// WHEN:  100 nodes are inserted with `on_add_node`
/// THEN:  Lookups return exactly the correct node IDs for each value
#[test]
fn test_property_index_auto_populated_on_node_insert() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "age");
    index.create_index("Person", "city");

    let cities = ["Paris", "London", "Berlin", "Tokyo", "NYC"];

    // Insert 100 nodes.
    for i in 0u64..100 {
        let age = 10 + i; // ages 10..109
        let city = cities[i as usize % cities.len()];
        let mut props = HashMap::new();
        props.insert("age".to_string(), serde_json::json!(age));
        props.insert("city".to_string(), serde_json::json!(city));
        index.on_add_node("Person", i, &props);
    }

    // THEN: verify index has entries for (Person, age) and (Person, city).
    assert!(
        index.has_index("Person", "age"),
        "age index should exist"
    );
    assert!(
        index.has_index("Person", "city"),
        "city index should exist"
    );

    // Verify lookup: node 0 has age=10.
    let age_10_nodes = index.lookup("Person", "age", &serde_json::json!(10));
    assert!(
        age_10_nodes.is_some_and(|b| b.contains(0)),
        "node 0 with age=10 should be in index"
    );

    // Verify lookup: node 42 has age=52.
    let age_52_nodes = index.lookup("Person", "age", &serde_json::json!(52));
    assert!(
        age_52_nodes.is_some_and(|b| b.contains(42)),
        "node 42 with age=52 should be in index"
    );

    // Verify city grouping: nodes 0, 5, 10, 15, ... have city "Paris".
    let paris_nodes = index.lookup("Person", "city", &serde_json::json!("Paris"));
    let paris_bitmap = paris_nodes.expect("test: Paris should have entries");
    assert_eq!(
        paris_bitmap.len(),
        20,
        "20 nodes (0,5,10,...,95) should have city=Paris"
    );
    for n in (0u32..100).step_by(5) {
        assert!(
            paris_bitmap.contains(n),
            "node {n} should have city=Paris"
        );
    }

    // Verify an unknown value returns None (no entries).
    let unknown = index.lookup("Person", "city", &serde_json::json!("Atlantis"));
    assert!(
        unknown.is_none(),
        "Atlantis should not be in the index"
    );

    // Verify an un-indexed property returns None.
    let not_indexed = index.lookup("Person", "email", &serde_json::json!("test@example.com"));
    assert!(
        not_indexed.is_none(),
        "email is not indexed, lookup should return None"
    );
}

// =========================================================================
// B. RangeIndex: range queries return correct results
// =========================================================================

/// GIVEN: A `RangeIndex` populated with 100 nodes (age: 10..110)
/// WHEN:  Range queries (>, <, BETWEEN) are executed
/// THEN:  Only nodes matching the range predicate are returned
#[test]
fn test_range_query_returns_correct_results_with_index() {
    let mut index = RangeIndex::new();
    index.create_index("Person", "age");

    // Insert 100 nodes with ages 10..109.
    for i in 0u64..100 {
        let age = 10 + i;
        index.insert("Person", "age", &serde_json::json!(age), i);
    }

    // GT: age > 100 should return nodes with ages 101..109 (IDs 91..99).
    let gt_100 = index.range_greater_than("Person", "age", &serde_json::json!(100));
    assert_eq!(gt_100.len(), 9, "ages 101..109 = 9 nodes");
    for expected_id in 91u32..100 {
        assert!(
            gt_100.contains(expected_id),
            "node {expected_id} (age {}) should be in gt(100)",
            10 + u64::from(expected_id)
        );
    }

    // LT: age < 15 should return nodes with ages 10..14 (IDs 0..4).
    let lt_15 = index.range_less_than("Person", "age", &serde_json::json!(15));
    assert_eq!(lt_15.len(), 5, "ages 10..14 = 5 nodes");
    for expected_id in 0u32..5 {
        assert!(
            lt_15.contains(expected_id),
            "node {expected_id} should be in lt(15)"
        );
    }

    // BETWEEN: 50 <= age <= 60 should return IDs 40..50 (ages 50..60 inclusive).
    let between = index.range_between(
        "Person",
        "age",
        &serde_json::json!(50),
        &serde_json::json!(60),
    );
    assert_eq!(between.len(), 11, "ages 50..60 inclusive = 11 nodes");

    // GTE: age >= 109 should return exactly 1 node (ID 99, age 109).
    let gte_109 = index.range_greater_or_equal("Person", "age", &serde_json::json!(109));
    assert_eq!(gte_109.len(), 1, "only one node has age >= 109");
    assert!(gte_109.contains(99), "node 99 has age 109");

    // Empty range: age > 200 should return nothing.
    let empty = index.range_greater_than("Person", "age", &serde_json::json!(200));
    assert!(empty.is_empty(), "no nodes have age > 200");
}

// =========================================================================
// C. Graph MATCH + WHERE: correct results via payload filtering
// =========================================================================

/// GIVEN: Graph collection with 20 nodes having {category: "A"|"B", status:
///        "active"|"inactive"} and edges connecting them
/// WHEN:  MATCH query filters by WHERE n.category = 'A' AND n.status = 'active'
/// THEN:  Only matching nodes are returned
#[test]
fn test_composite_property_filter_via_match() {
    let (_dir, _db, gc) = setup_graph_db();

    // Insert 20 labeled nodes with category and status.
    let mut expected_active_a: HashSet<u64> = HashSet::new();
    for i in 1u64..=20 {
        let category = if i % 2 == 0 { "A" } else { "B" };
        let status = if i % 3 == 0 { "inactive" } else { "active" };
        let payload = serde_json::json!({
            "_labels": ["Item"],
            "category": category,
            "status": status,
        });
        gc.upsert_node_payload(i, &payload)
            .expect("test: upsert node payload");
        if category == "A" && status == "active" {
            expected_active_a.insert(i);
        }
    }

    // Connect nodes sequentially: 1->2, 2->3, ..., 19->20.
    for i in 1u64..20 {
        let edge = GraphEdge::new(i, i, i + 1, "NEXT")
            .expect("test: create edge");
        gc.add_edge(edge).expect("test: add edge");
    }

    // Execute MATCH (n:Item)-[]->(m) WHERE n.category = 'A' AND n.status = 'active'.
    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesdb_core::velesql::NodePattern::new()
                    .with_alias("n")
                    .with_label("Item"),
                velesdb_core::velesql::NodePattern::new().with_alias("m"),
            ],
            relationships: vec![velesdb_core::velesql::RelationshipPattern::new(
                velesdb_core::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: Some(velesdb_core::velesql::Condition::And(
            Box::new(velesdb_core::velesql::Condition::Comparison(
                velesdb_core::velesql::Comparison {
                    column: "n.category".to_string(),
                    operator: velesdb_core::velesql::CompareOp::Eq,
                    value: velesdb_core::velesql::Value::String("A".to_string()),
                },
            )),
            Box::new(velesdb_core::velesql::Condition::Comparison(
                velesdb_core::velesql::Comparison {
                    column: "n.status".to_string(),
                    operator: velesdb_core::velesql::CompareOp::Eq,
                    value: velesdb_core::velesql::Value::String("active".to_string()),
                },
            )),
        )),
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![velesdb_core::velesql::ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH should succeed");

    // All start nodes (bound to "n") must be in our expected set.
    let start_ids: HashSet<u64> = results
        .iter()
        .filter_map(|r| r.bindings.get("n").copied())
        .collect();

    // Every start node must satisfy category=A AND status=active.
    for &id in &start_ids {
        assert!(
            expected_active_a.contains(&id),
            "start node {id} should be category=A AND status=active"
        );
    }

    // Verify we found at least some results (sanity).
    assert!(
        !results.is_empty(),
        "MATCH should find at least one path"
    );
}

// =========================================================================
// D. PropertyIndex survives node update (not stale)
// =========================================================================

/// GIVEN: A property index with node {age: 25}
/// WHEN:  Node is updated to {age: 30} via `on_update_property`
/// THEN:  Lookup for age=25 does NOT return the node
/// AND:   Lookup for age=30 DOES return the node
#[test]
fn test_property_index_survives_node_update() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "age");

    // Initial insert.
    let mut props = HashMap::new();
    props.insert("age".to_string(), serde_json::json!(25));
    index.on_add_node("Person", 1, &props);

    // Verify initial state.
    assert!(
        index
            .lookup("Person", "age", &serde_json::json!(25))
            .is_some_and(|b| b.contains(1)),
        "node 1 should be indexed under age=25"
    );

    // Update: age 25 -> 30.
    index.on_update_property(
        "Person",
        1,
        "age",
        &serde_json::json!(25),
        &serde_json::json!(30),
    );

    // Old value should NOT contain node 1.
    let old_lookup = index.lookup("Person", "age", &serde_json::json!(25));
    let old_contains = old_lookup.is_some_and(|b| b.contains(1));
    assert!(
        !old_contains,
        "node 1 should NOT be in age=25 after update"
    );

    // New value SHOULD contain node 1.
    assert!(
        index
            .lookup("Person", "age", &serde_json::json!(30))
            .is_some_and(|b| b.contains(1)),
        "node 1 should be in age=30 after update"
    );
}

// =========================================================================
// E. RangeIndex: update consistency
// =========================================================================

/// GIVEN: A range index with node {age: 25}
/// WHEN:  Node value is removed and re-inserted with age=30
/// AND:   Range query WHERE age > 28
/// THEN:  Updated node IS in results (index was updated, not stale)
#[test]
fn test_range_index_survives_node_update() {
    let mut index = RangeIndex::new();
    index.create_index("Person", "age");

    // Insert node 1 with age=25.
    index.insert("Person", "age", &serde_json::json!(25), 1);

    // Verify: age > 28 should NOT include node 1.
    let before = index.range_greater_than("Person", "age", &serde_json::json!(28));
    assert!(
        !before.contains(1),
        "node 1 (age=25) should NOT be in age>28"
    );

    // Update: remove old value, insert new value.
    index.remove("Person", "age", &serde_json::json!(25), 1);
    index.insert("Person", "age", &serde_json::json!(30), 1);

    // Verify: age > 28 SHOULD include node 1 now.
    let after = index.range_greater_than("Person", "age", &serde_json::json!(28));
    assert!(
        after.contains(1),
        "node 1 (age=30) should be in age>28 after update"
    );

    // Verify: age > 30 should NOT include node 1 (exclusive).
    let exclusive = index.range_greater_than("Person", "age", &serde_json::json!(30));
    assert!(
        !exclusive.contains(1),
        "node 1 (age=30) should NOT be in age>30 (exclusive)"
    );
}

// =========================================================================
// F. PropertyIndex: cardinality and memory tracking
// =========================================================================

/// GIVEN: A property index with data for multiple values
/// WHEN:  `cardinality` and `memory_usage` are queried
/// THEN:  Cardinality reflects the number of distinct values
/// AND:   Memory usage is non-zero
#[test]
fn test_property_index_cardinality_and_memory() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "city");

    let cities = ["Paris", "London", "Berlin"];
    for (i, city) in cities.iter().enumerate() {
        let mut props = HashMap::new();
        props.insert("city".to_string(), serde_json::json!(city));
        // Two nodes per city.
        index.on_add_node("Person", (i * 2) as u64, &props);
        index.on_add_node("Person", (i * 2 + 1) as u64, &props);
    }

    // Cardinality = 3 distinct city values.
    assert_eq!(
        index.cardinality("Person", "city"),
        Some(3),
        "3 distinct cities should yield cardinality 3"
    );

    // Memory usage should be non-zero.
    assert!(
        index.memory_usage() > 0,
        "index with data should have non-zero memory usage"
    );

    // Drop the index.
    assert!(
        index.drop_index("Person", "city"),
        "dropping existing index should return true"
    );
    assert!(
        !index.has_index("Person", "city"),
        "index should not exist after drop"
    );

    // Dropping again should return false.
    assert!(
        !index.drop_index("Person", "city"),
        "dropping non-existent index should return false"
    );
}

// =========================================================================
// G. Graph MATCH with WHERE range filter (GT) via payload scan
// =========================================================================

/// GIVEN: Graph collection with 10 nodes (age: 10, 20, ..., 100)
///        and sequential edges
/// WHEN:  MATCH query with WHERE n.age > 50
/// THEN:  Only nodes with age > 50 are returned as start nodes
#[test]
fn test_match_where_range_filter_gt() {
    let (_dir, _db, gc) = setup_graph_db();

    // Insert 10 nodes with ages 10, 20, ..., 100.
    for i in 1u64..=10 {
        let age = i * 10;
        let payload = serde_json::json!({
            "_labels": ["Person"],
            "age": age,
            "name": format!("person_{i}"),
        });
        gc.upsert_node_payload(i, &payload)
            .expect("test: upsert node payload");
    }

    // Connect: 1->2, 2->3, ..., 9->10.
    for i in 1u64..10 {
        let edge = GraphEdge::new(i, i, i + 1, "NEXT")
            .expect("test: create edge");
        gc.add_edge(edge).expect("test: add edge");
    }

    // MATCH (n:Person)-[]->(m) WHERE n.age > 50
    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesdb_core::velesql::NodePattern::new()
                    .with_alias("n")
                    .with_label("Person"),
                velesdb_core::velesql::NodePattern::new().with_alias("m"),
            ],
            relationships: vec![velesdb_core::velesql::RelationshipPattern::new(
                velesdb_core::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: Some(velesdb_core::velesql::Condition::Comparison(
            velesdb_core::velesql::Comparison {
                column: "n.age".to_string(),
                operator: velesdb_core::velesql::CompareOp::Gt,
                value: velesdb_core::velesql::Value::Integer(50),
            },
        )),
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![velesdb_core::velesql::ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH should succeed");

    // All start node bindings "n" should have age > 50.
    let start_ids: HashSet<u64> = results
        .iter()
        .filter_map(|r| r.bindings.get("n").copied())
        .collect();

    // Nodes with age > 50 are IDs 6, 7, 8, 9 (ages 60, 70, 80, 90).
    // Node 10 (age 100) has no outgoing edge, so it will not appear.
    let expected: HashSet<u64> = [6, 7, 8, 9].into_iter().collect();
    assert_eq!(
        start_ids, expected,
        "only nodes 6-9 (age 60-90) have outgoing edges AND age > 50"
    );

    // Verify no start node has age <= 50.
    for &id in &start_ids {
        let age = id * 10;
        assert!(age > 50, "start node {id} has age {age}, should be > 50");
    }
}

// =========================================================================
// H. Label index auto-population verified via MATCH
// =========================================================================

/// GIVEN: Graph collection with nodes having different labels
/// WHEN:  MATCH filters by label (n:Person)
/// THEN:  Only Person-labeled nodes are used as start nodes
/// This tests that the `LabelIndex` in `store_node_payload` is auto-populated.
#[test]
fn test_label_index_auto_populated_on_insert() {
    let (_dir, _db, gc) = setup_graph_db();

    // Insert 5 Person nodes and 5 Company nodes.
    for i in 1u64..=5 {
        gc.upsert_node_payload(
            i,
            &serde_json::json!({"_labels": ["Person"], "name": format!("person_{i}")}),
        )
        .expect("test: upsert Person");
    }
    for i in 6u64..=10 {
        gc.upsert_node_payload(
            i,
            &serde_json::json!({"_labels": ["Company"], "name": format!("company_{i}")}),
        )
        .expect("test: upsert Company");
    }

    // Edges: each Person -> next Company.
    for i in 1u64..=5 {
        let edge = GraphEdge::new(i, i, i + 5, "WORKS_AT")
            .expect("test: create edge");
        gc.add_edge(edge).expect("test: add edge");
    }

    // MATCH (p:Person)-[]->(c)
    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesdb_core::velesql::NodePattern::new()
                    .with_alias("p")
                    .with_label("Person"),
                velesdb_core::velesql::NodePattern::new().with_alias("c"),
            ],
            relationships: vec![velesdb_core::velesql::RelationshipPattern::new(
                velesdb_core::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![velesdb_core::velesql::ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH should succeed");

    // All start nodes must be Person nodes (IDs 1..=5).
    let start_ids: HashSet<u64> = results
        .iter()
        .filter_map(|r| r.bindings.get("p").copied())
        .collect();

    assert_eq!(start_ids.len(), 5, "all 5 Person nodes should match");
    for id in 1u64..=5 {
        assert!(
            start_ids.contains(&id),
            "Person node {id} should be a start node"
        );
    }

    // No Company node should appear as start.
    for id in 6u64..=10 {
        assert!(
            !start_ids.contains(&id),
            "Company node {id} should NOT be a start node for (p:Person)"
        );
    }
}

// =========================================================================
// I. PropertyIndex: remove on node delete
// =========================================================================

/// GIVEN: A property index with multiple nodes
/// WHEN:  A node is removed via `on_remove_node`
/// THEN:  The index no longer contains the removed node
/// AND:   Other nodes are unaffected
#[test]
fn test_property_index_remove_on_delete() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "city");

    // Insert two nodes.
    let mut props_alice = HashMap::new();
    props_alice.insert("city".to_string(), serde_json::json!("Paris"));
    index.on_add_node("Person", 1, &props_alice);

    let mut props_bob = HashMap::new();
    props_bob.insert("city".to_string(), serde_json::json!("Paris"));
    index.on_add_node("Person", 2, &props_bob);

    // Both should be in the index.
    let before = index.lookup("Person", "city", &serde_json::json!("Paris"));
    assert!(
        before.is_some_and(|b| b.contains(1) && b.contains(2)),
        "both nodes should be indexed under Paris"
    );

    // Remove node 1.
    index.on_remove_node("Person", 1, &props_alice);

    // Node 1 should be gone, node 2 should remain.
    let after = index.lookup("Person", "city", &serde_json::json!("Paris"));
    let after_bitmap = after.expect("test: Paris should still have entries");
    assert!(
        !after_bitmap.contains(1),
        "node 1 should be removed from index"
    );
    assert!(
        after_bitmap.contains(2),
        "node 2 should still be in index"
    );
}

// =========================================================================
// J. Negative: non-indexed property lookup returns None
// =========================================================================

/// GIVEN: A property index with only ("Person", "age") indexed
/// WHEN:  Lookup on ("Person", "email") or ("Animal", "age")
/// THEN:  Returns None (not Some with empty bitmap)
#[test]
fn test_property_index_unindexed_lookup_returns_none() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "age");

    let mut props = HashMap::new();
    props.insert("age".to_string(), serde_json::json!(25));
    index.on_add_node("Person", 1, &props);

    // Wrong property.
    assert!(
        index.lookup("Person", "email", &serde_json::json!("test")).is_none(),
        "un-indexed property should return None"
    );

    // Wrong label.
    assert!(
        index.lookup("Animal", "age", &serde_json::json!(25)).is_none(),
        "un-indexed label should return None"
    );
}

// =========================================================================
// K. Negative: MATCH on non-existent label returns empty results
// =========================================================================

/// GIVEN: Graph collection with only "Person" nodes
/// WHEN:  MATCH (n:NonExistent)-[]->(m)
/// THEN:  Returns empty results
#[test]
fn test_match_nonexistent_label_returns_empty() {
    let (_dir, _db, gc) = setup_graph_db();

    gc.upsert_node_payload(
        1,
        &serde_json::json!({"_labels": ["Person"], "name": "Alice"}),
    )
    .expect("test: upsert");
    gc.upsert_node_payload(
        2,
        &serde_json::json!({"_labels": ["Person"], "name": "Bob"}),
    )
    .expect("test: upsert");

    let edge = GraphEdge::new(1, 1, 2, "KNOWS").expect("test: create edge");
    gc.add_edge(edge).expect("test: add edge");

    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesdb_core::velesql::NodePattern::new()
                    .with_alias("n")
                    .with_label("NonExistent"),
                velesdb_core::velesql::NodePattern::new().with_alias("m"),
            ],
            relationships: vec![velesdb_core::velesql::RelationshipPattern::new(
                velesdb_core::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH should succeed but return empty");

    assert!(
        results.is_empty(),
        "MATCH on non-existent label should return empty results"
    );
}

// =========================================================================
// L. PropertyIndex persistence round-trip
// =========================================================================

/// GIVEN: A property index populated with data
/// WHEN:  Serialized to bytes and deserialized back
/// THEN:  All data is preserved
#[test]
fn test_property_index_persistence_round_trip() {
    let mut index = PropertyIndex::new();
    index.create_index("Person", "age");
    index.create_index("Person", "city");

    let mut props = HashMap::new();
    props.insert("age".to_string(), serde_json::json!(25));
    props.insert("city".to_string(), serde_json::json!("Paris"));
    index.on_add_node("Person", 1, &props);

    // Serialize and deserialize.
    let bytes = index.to_bytes().expect("test: serialization should succeed");
    let restored =
        PropertyIndex::from_bytes(&bytes).expect("test: deserialization should succeed");

    // Verify data survives round-trip.
    assert!(
        restored.has_index("Person", "age"),
        "age index should survive round-trip"
    );
    assert!(
        restored.has_index("Person", "city"),
        "city index should survive round-trip"
    );
    assert!(
        restored
            .lookup("Person", "age", &serde_json::json!(25))
            .is_some_and(|b| b.contains(1)),
        "node 1 with age=25 should survive round-trip"
    );
    assert!(
        restored
            .lookup("Person", "city", &serde_json::json!("Paris"))
            .is_some_and(|b| b.contains(1)),
        "node 1 with city=Paris should survive round-trip"
    );
}

// =========================================================================
// M. RangeIndex persistence round-trip
// =========================================================================

/// GIVEN: A range index populated with data
/// WHEN:  Serialized to bytes and deserialized back
/// THEN:  Range queries still return correct results
#[test]
fn test_range_index_persistence_round_trip() {
    let mut index = RangeIndex::new();
    index.create_index("Event", "timestamp");

    index.insert("Event", "timestamp", &serde_json::json!(100), 1);
    index.insert("Event", "timestamp", &serde_json::json!(200), 2);
    index.insert("Event", "timestamp", &serde_json::json!(300), 3);

    // Serialize and deserialize.
    let bytes = index.to_bytes().expect("test: serialization should succeed");
    let restored =
        RangeIndex::from_bytes(&bytes).expect("test: deserialization should succeed");

    // Range query on restored index.
    let gt_150 = restored.range_greater_than("Event", "timestamp", &serde_json::json!(150));
    assert_eq!(gt_150.len(), 2, "timestamps 200, 300 > 150");
    assert!(gt_150.contains(2), "node 2 (ts=200) should be in result");
    assert!(gt_150.contains(3), "node 3 (ts=300) should be in result");
}

// =========================================================================
// N. Graph collection MATCH with WHERE + label filter combined
// =========================================================================

/// GIVEN: Graph with mixed-label nodes and varied properties
/// WHEN:  MATCH (n:Person)-[]->(m) WHERE n.age > 25
/// THEN:  Only Person nodes with age > 25 that have outgoing edges are returned
/// This tests the combined label index + WHERE payload evaluation pipeline.
#[test]
fn test_match_label_plus_where_combined() {
    let (_dir, _db, gc) = setup_graph_db();

    // Insert Person nodes (IDs 1-5) with ages 20, 25, 30, 35, 40.
    for i in 1u64..=5 {
        let age = 15 + i * 5; // 20, 25, 30, 35, 40
        gc.upsert_node_payload(
            i,
            &serde_json::json!({"_labels": ["Person"], "age": age, "name": format!("p{i}")}),
        )
        .expect("test: upsert Person");
    }

    // Insert Company nodes (IDs 6-8).
    for i in 6u64..=8 {
        gc.upsert_node_payload(
            i,
            &serde_json::json!({"_labels": ["Company"], "name": format!("c{i}")}),
        )
        .expect("test: upsert Company");
    }

    // Edges: Person -> Company.
    gc.add_edge(GraphEdge::new(1, 1, 6, "WORKS_AT").expect("test: edge"))
        .expect("test: add edge");
    gc.add_edge(GraphEdge::new(2, 2, 6, "WORKS_AT").expect("test: edge"))
        .expect("test: add edge");
    gc.add_edge(GraphEdge::new(3, 3, 7, "WORKS_AT").expect("test: edge"))
        .expect("test: add edge");
    gc.add_edge(GraphEdge::new(4, 4, 7, "WORKS_AT").expect("test: edge"))
        .expect("test: add edge");
    gc.add_edge(GraphEdge::new(5, 5, 8, "WORKS_AT").expect("test: edge"))
        .expect("test: add edge");

    // MATCH (n:Person)-[]->(m) WHERE n.age > 25
    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesdb_core::velesql::NodePattern::new()
                    .with_alias("n")
                    .with_label("Person"),
                velesdb_core::velesql::NodePattern::new().with_alias("m"),
            ],
            relationships: vec![velesdb_core::velesql::RelationshipPattern::new(
                velesdb_core::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: Some(velesdb_core::velesql::Condition::Comparison(
            velesdb_core::velesql::Comparison {
                column: "n.age".to_string(),
                operator: velesdb_core::velesql::CompareOp::Gt,
                value: velesdb_core::velesql::Value::Integer(25),
            },
        )),
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![velesdb_core::velesql::ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH should succeed");

    let start_ids: HashSet<u64> = results
        .iter()
        .filter_map(|r| r.bindings.get("n").copied())
        .collect();

    // Expected: nodes 3 (age 30), 4 (age 35), 5 (age 40).
    // Node 1 (age 20) and node 2 (age 25) should be filtered out.
    let expected: HashSet<u64> = [3, 4, 5].into_iter().collect();
    assert_eq!(
        start_ids, expected,
        "only Person nodes with age > 25 and outgoing edges should match"
    );
}
