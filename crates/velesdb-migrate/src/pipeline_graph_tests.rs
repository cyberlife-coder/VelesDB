use super::*;
use crate::connectors::ExtractedPoint;

fn make_point(id: &str, payload: serde_json::Value) -> ExtractedPoint {
    let payload_map = payload
        .as_object()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    ExtractedPoint {
        id: id.to_string(),
        vector: vec![],
        payload: payload_map,
        sparse_vector: None,
    }
}

fn make_relation(from: &str, label: &str) -> RelationConfig {
    RelationConfig {
        from_column: from.to_string(),
        to_table: "target".to_string(),
        to_column: "id".to_string(),
        edge_label: label.to_string(),
        weight_column: None,
    }
}

fn make_weighted_relation(from: &str, label: &str, weight_col: &str) -> RelationConfig {
    RelationConfig {
        weight_column: Some(weight_col.to_string()),
        ..make_relation(from, label)
    }
}

#[test]
fn test_build_edge_string_fk() {
    let point = make_point("doc-1", serde_json::json!({"author_id": "auth-42"}));
    let relation = make_relation("author_id", "AUTHORED_BY");
    let edge = build_edge(&point, &relation);
    assert!(edge.is_some());
    let e = edge.expect("test: edge should be Some");
    assert_eq!(e.source(), stable_point_id("doc-1"));
    assert_eq!(e.target(), stable_point_id("auth-42"));
}

#[test]
fn test_build_edge_numeric_fk() {
    let point = make_point("99", serde_json::json!({"category_id": 7}));
    let relation = make_relation("category_id", "BELONGS_TO");
    let edge = build_edge(&point, &relation);
    assert!(edge.is_some());
    assert_eq!(
        edge.expect("test: edge should be Some").source(),
        stable_point_id("99")
    );
}

#[test]
fn test_build_edge_missing_fk_returns_none() {
    let point = make_point("1", serde_json::json!({}));
    let relation = make_relation("author_id", "AUTHORED_BY");
    assert!(build_edge(&point, &relation).is_none());
}

#[test]
fn test_build_edge_string_fk_deterministic_id() {
    // GIVEN: same point and relation
    let point = make_point("doc-1", serde_json::json!({"author_id": "auth-42"}));
    let relation = make_relation("author_id", "AUTHORED_BY");

    // WHEN: build_edge is called twice
    let e1 = build_edge(&point, &relation).expect("test: e1 should be Some");
    let e2 = build_edge(&point, &relation).expect("test: e2 should be Some");

    // THEN: both produce the same deterministic ID
    assert_eq!(
        e1.id(),
        e2.id(),
        "Edge IDs must be deterministic for the same input"
    );
}

#[test]
fn test_build_edge_different_labels_produce_different_ids() {
    // GIVEN: same point but different edge labels
    let point = make_point("doc-1", serde_json::json!({"author_id": "auth-42"}));
    let rel1 = make_relation("author_id", "AUTHORED_BY");
    let rel2 = make_relation("author_id", "EDITED_BY");

    // WHEN: build_edge is called with each relation
    let e1 = build_edge(&point, &rel1).expect("test: e1 should be Some");
    let e2 = build_edge(&point, &rel2).expect("test: e2 should be Some");

    // THEN: different labels produce different IDs
    assert_ne!(
        e1.id(),
        e2.id(),
        "Different edge labels must produce different IDs"
    );
}

#[test]
fn test_build_edge_attaches_numeric_weight_property() {
    // GIVEN: a relation with a weight_column whose value is numeric
    let point = make_point(
        "doc-1",
        serde_json::json!({"author_id": "auth-42", "score": 0.75}),
    );
    let relation = make_weighted_relation("author_id", "AUTHORED_BY", "score");

    // WHEN: build_edge runs the weighted branch of attach_weight
    let edge = build_edge(&point, &relation).expect("test: edge should be Some");

    // THEN: the weight is attached as an edge property
    let weight = edge
        .property("weight")
        .expect("test: weight property should be present");
    assert_eq!(weight, &serde_json::json!(0.75));
}

#[test]
fn test_build_edge_attaches_integer_weight_as_f64() {
    // GIVEN: a relation whose weight column holds a JSON integer
    let point = make_point("doc-9", serde_json::json!({"ref_id": "tgt-9", "rank": 3}));
    let relation = make_weighted_relation("ref_id", "LINKS_TO", "rank");

    // WHEN: build_edge runs (integer is coerced via as_f64)
    let edge = build_edge(&point, &relation).expect("test: edge should be Some");

    // THEN: the integer weight is stored as a float
    let weight = edge
        .property("weight")
        .expect("test: weight property should be present");
    assert_eq!(
        weight.as_f64().expect("test: weight should be numeric"),
        3.0
    );
}

#[test]
fn test_build_edge_weight_column_missing_value_skips_property() {
    // GIVEN: a weight_column configured but absent from the point payload
    let point = make_point("doc-2", serde_json::json!({"author_id": "auth-7"}));
    let relation = make_weighted_relation("author_id", "AUTHORED_BY", "score");

    // WHEN: build_edge runs the second early-return branch of attach_weight
    let edge = build_edge(&point, &relation).expect("test: edge should be Some");

    // THEN: no weight property is attached and properties stay empty
    assert!(edge.property("weight").is_none());
    assert!(edge.properties().is_empty());
}

#[test]
fn test_build_edge_weight_non_numeric_skips_property() {
    // GIVEN: a weight_column present but holding a non-numeric (string) value
    let point = make_point(
        "doc-3",
        serde_json::json!({"author_id": "auth-8", "score": "high"}),
    );
    let relation = make_weighted_relation("author_id", "AUTHORED_BY", "score");

    // WHEN: build_edge runs (as_f64 returns None for a string)
    let edge = build_edge(&point, &relation).expect("test: edge should be Some");

    // THEN: the non-numeric value is ignored, no weight property attached
    assert!(edge.property("weight").is_none());
    assert!(edge.properties().is_empty());
}

#[test]
fn seed_retries_failed_node_on_next_occurrence() {
    // Regression guard: a node whose stub-seed upsert fails must NOT be
    // marked as seeded, so a later edge referencing the same node id
    // retries the upsert instead of silently skipping it forever (which
    // would leave that node's edges permanently rejected by #1442's
    // add_edges_batch validation, since the endpoint was never actually
    // stored).
    let attempts = std::cell::RefCell::new(Vec::new());
    let mut seeded = std::collections::HashSet::new();

    let edge1 = velesdb_core::GraphEdge::new(1, 100, 200, "REL").expect("valid edge");
    let edge2 = velesdb_core::GraphEdge::new(2, 100, 300, "REL").expect("valid edge");

    // First occurrence of node 100: its upsert fails.
    seed_edge_endpoints(std::slice::from_ref(&edge1), &mut seeded, |id| {
        attempts.borrow_mut().push(id);
        if id == 100 {
            Err("simulated storage failure")
        } else {
            Ok(())
        }
    });
    assert!(
        !seeded.contains(&100),
        "a failed upsert must not mark the node as seeded"
    );
    assert!(seeded.contains(&200), "the succeeding endpoint is seeded");

    // Second occurrence of node 100 (via edge2): must retry since it was
    // never marked seeded.
    seed_edge_endpoints(std::slice::from_ref(&edge2), &mut seeded, |id| {
        attempts.borrow_mut().push(id);
        Ok::<(), &str>(())
    });
    assert!(
        seeded.contains(&100),
        "node 100 must be seeded after a successful retry"
    );
    assert_eq!(
        *attempts.borrow(),
        vec![100, 200, 100, 300],
        "node 100's upsert must be attempted again on its next occurrence"
    );
}
