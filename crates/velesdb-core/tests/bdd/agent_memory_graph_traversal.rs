//! BDD tests for the AGENT-MEMORY graph dimension: `relate` / `relations` /
//! `unrelate` over `SemanticMemory`, plus VelesQL `MATCH` / `NOT MATCH`
//! traversal over the queryable `_semantic_memory` collection.
//!
//! Every test pins an EXACT result computed by hand from a fixed graph:
//!
//! ```text
//!   1 -[:SUPPORTS    {weight: 0.9}]-> 2
//!   2 -[:SUPPORTS    {weight: 0.4}]-> 3
//!   1 -[:CONTRADICTS]               -> 4
//! ```
//!
//! `relations(id)` returns OUTGOING edges only. `SELECT * ... WHERE MATCH
//! (a)-[:REL]->(b)` returns the SOURCE/anchor facts that have a matching
//! outgoing edge (proven semantics, see `graph_vector_hybrid.rs`); `NOT MATCH`
//! returns the exact complement among the four stored facts.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tempfile::TempDir;
use velesdb_core::agent::AgentMemory;
use velesdb_core::{velesql::Parser, Database, SearchResult};

use super::helpers::{execute_sql, result_ids};

// ============================================================================
// Shared setup — the fixed 4-fact memory graph above
// ============================================================================

/// Builds the canonical graph and returns `(dir, db, memory, edge_id_1_2)`.
///
/// Facts 1..=4 are well-separated cosine anchors. Edge ids are explicit so
/// `unrelate` tests can target one precisely. `weight` properties live on the
/// two SUPPORTS edges so property-filtered MATCH has ground truth.
fn setup_memory_graph() -> (TempDir, Arc<Database>, AgentMemory, u64) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Arc::new(Database::open(dir.path()).expect("test: open database"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: AgentMemory");
    let sm = memory.semantic();
    sm.store(1, "rust is fast", &[1.0, 0.0, 0.0, 0.0])
        .expect("store 1");
    sm.store(2, "rust is safe", &[1.0, 0.3, 0.0, 0.0])
        .expect("store 2");
    sm.store(3, "go has gc", &[1.0, 0.7, 0.0, 0.0])
        .expect("store 3");
    sm.store(4, "rust is slow", &[1.0, 3.0, 0.0, 0.0])
        .expect("store 4");
    let mut w9 = serde_json::Map::new();
    w9.insert("weight".to_string(), serde_json::json!(0.9));
    let mut w4 = serde_json::Map::new();
    w4.insert("weight".to_string(), serde_json::json!(0.4));
    let e12 = sm.relate(1, 2, "SUPPORTS", Some(&w9)).expect("relate 1->2");
    sm.relate(2, 3, "SUPPORTS", Some(&w4)).expect("relate 2->3");
    sm.relate(1, 4, "CONTRADICTS", None).expect("relate 1->4");
    (dir, db, memory, e12)
}

/// Collects `(target_id, label)` pairs from a slice of edges, order-independent.
fn edge_targets(edges: &[velesdb_core::GraphEdge]) -> HashSet<(u64, String)> {
    edges
        .iter()
        .map(|e| (e.target(), e.label().to_string()))
        .collect()
}

/// Runs a GraphFirst `MATCH` over `_semantic_memory` via the `_collection`
/// routing param (the path that binds relationship aliases and supports an
/// in-pattern `WHERE r.prop`). On this path `point.id` is the traversal TARGET.
fn run_match_on_semantic(db: &Database, sql: &str) -> Vec<SearchResult> {
    let query = Parser::parse(sql).expect("test: parse MATCH");
    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String("_semantic_memory".to_string()),
    );
    db.execute_query(&query, &params)
        .expect("test: execute GraphFirst MATCH over _semantic_memory")
}

// ============================================================================
// 1. relations() — exact set of outgoing edges
// ============================================================================

/// GIVEN fact 1 with outgoing 1-[:SUPPORTS]->2 and 1-[:CONTRADICTS]->4
/// WHEN `relations(1)` is read
/// THEN exactly those two outgoing edges are returned (target+label), and the
///      incoming-only / two-hop edges (2->3) are NOT included — `relations`
///      is strictly outgoing.
#[test]
fn test_relations_returns_exact_outgoing_set_of_fact_1() {
    let (_dir, _db, memory, _e12) = setup_memory_graph();

    let rels = memory.semantic().relations(1).expect("relations(1)");

    assert_eq!(rels.len(), 2, "fact 1 has exactly two outgoing edges");
    assert_eq!(
        edge_targets(&rels),
        [(2, "SUPPORTS".to_string()), (4, "CONTRADICTS".to_string())]
            .into_iter()
            .collect(),
        "outgoing edges of fact 1 must be SUPPORTS->2 and CONTRADICTS->4"
    );
}

/// GIVEN fact 2 with a single outgoing 2-[:SUPPORTS]->3 (and an INCOMING 1->2)
/// WHEN `relations(2)` is read
/// THEN only the outgoing edge to 3 appears; the incoming 1->2 edge is excluded.
#[test]
fn test_relations_excludes_incoming_edges() {
    let (_dir, _db, memory, _e12) = setup_memory_graph();

    let rels = memory.semantic().relations(2).expect("relations(2)");

    assert_eq!(rels.len(), 1, "fact 2 has exactly one outgoing edge");
    assert_eq!(rels[0].target(), 3, "the only outgoing target is fact 3");
    assert_eq!(rels[0].label(), "SUPPORTS");
}

/// GIVEN leaf facts 3 and 4 (no outgoing edges)
/// WHEN `relations(3)` and `relations(4)` are read
/// THEN both are empty — only incoming edges reach them.
#[test]
fn test_relations_of_leaf_facts_are_empty() {
    let (_dir, _db, memory, _e12) = setup_memory_graph();

    assert!(
        memory
            .semantic()
            .relations(3)
            .expect("relations(3)")
            .is_empty(),
        "fact 3 has no outgoing edges"
    );
    assert!(
        memory
            .semantic()
            .relations(4)
            .expect("relations(4)")
            .is_empty(),
        "fact 4 has no outgoing edges"
    );
}

// ============================================================================
// 2. relate -> usable edge_id; unrelate shrinks relations to the exact set
// ============================================================================

/// GIVEN the edge id returned by `relate(1, 2, SUPPORTS)`
/// WHEN it is passed to `unrelate`
/// THEN `unrelate` returns true and `relations(1)` shrinks to exactly the
///      remaining CONTRADICTS->4 edge (the SUPPORTS->2 edge is gone).
#[test]
fn test_unrelate_returns_true_and_relations_shrink_to_exact_set() {
    let (_dir, _db, memory, e12) = setup_memory_graph();
    let sm = memory.semantic();

    assert!(sm.unrelate(e12).expect("unrelate e12"), "edge existed");

    let rels = sm.relations(1).expect("relations(1) after unrelate");
    assert_eq!(rels.len(), 1, "only the CONTRADICTS edge remains on fact 1");
    assert_eq!(rels[0].target(), 4);
    assert_eq!(rels[0].label(), "CONTRADICTS");
}

/// GIVEN an edge id that does not exist
/// WHEN `unrelate` is called twice on the same SUPPORTS->2 edge
/// THEN the first call returns true (removed), the second returns false
///      (already gone) — `unrelate` reports real removal, not a no-op success.
#[test]
fn test_unrelate_is_idempotent_reporting_first_true_then_false() {
    let (_dir, _db, memory, e12) = setup_memory_graph();
    let sm = memory.semantic();

    assert!(
        sm.unrelate(e12).expect("first unrelate"),
        "first removes it"
    );
    assert!(
        !sm.unrelate(e12).expect("second unrelate"),
        "second call finds nothing to remove"
    );
}

// ============================================================================
// 3 & 4. VelesQL MATCH / NOT MATCH over _semantic_memory
// ============================================================================

/// GIVEN edges 1-[:SUPPORTS]->2 and 2-[:SUPPORTS]->3
/// WHEN `SELECT * FROM _semantic_memory WHERE MATCH (a)-[:SUPPORTS]->(b)`
/// THEN exactly the SOURCE facts with an outgoing SUPPORTS edge are returned:
///      {1, 2}. Fact 3 (leaf) and fact 4 (only CONTRADICTS) are excluded.
#[test]
fn test_match_supports_returns_exact_source_anchor_set() {
    let (_dir, db, _memory, _e12) = setup_memory_graph();

    let results = execute_sql(
        &db,
        "SELECT * FROM _semantic_memory WHERE MATCH (a)-[:SUPPORTS]->(b) LIMIT 20",
    )
    .expect("MATCH over _semantic_memory must execute");

    assert_eq!(
        result_ids(&results),
        [1u64, 2].into_iter().collect(),
        "only facts with an outgoing SUPPORTS edge anchor the pattern"
    );
}

/// GIVEN the single edge 1-[:CONTRADICTS]->4
/// WHEN `SELECT * FROM _semantic_memory WHERE MATCH (a)-[:CONTRADICTS]->(b)`
/// THEN only fact 1 anchors the pattern (it is the sole CONTRADICTS source).
#[test]
fn test_match_contradicts_returns_single_source() {
    let (_dir, db, _memory, _e12) = setup_memory_graph();

    let results = execute_sql(
        &db,
        "SELECT * FROM _semantic_memory WHERE MATCH (a)-[:CONTRADICTS]->(b) LIMIT 20",
    )
    .expect("CONTRADICTS MATCH must execute");

    assert_eq!(
        result_ids(&results),
        [1u64].into_iter().collect(),
        "fact 1 is the only CONTRADICTS source"
    );
}

/// GIVEN the four stored facts and the SUPPORTS sources {1, 2}
/// WHEN `NOT MATCH (a)-[:SUPPORTS]->(b)` is applied
/// THEN the result is the EXACT complement {3, 4}: every stored fact appears
///      in exactly one of MATCH / NOT MATCH, and the two sets are disjoint and
///      union to all four facts (complement property).
#[test]
fn test_not_match_supports_is_exact_complement() {
    let (_dir, db, _memory, _e12) = setup_memory_graph();

    let matched = result_ids(
        &execute_sql(
            &db,
            "SELECT * FROM _semantic_memory WHERE MATCH (a)-[:SUPPORTS]->(b) LIMIT 20",
        )
        .expect("MATCH must execute"),
    );
    let unmatched = result_ids(
        &execute_sql(
            &db,
            "SELECT * FROM _semantic_memory WHERE NOT MATCH (a)-[:SUPPORTS]->(b) LIMIT 20",
        )
        .expect("NOT MATCH must execute"),
    );

    assert_eq!(
        unmatched,
        [3u64, 4].into_iter().collect(),
        "NOT MATCH yields the facts without an outgoing SUPPORTS edge"
    );
    assert!(
        matched.is_disjoint(&unmatched),
        "MATCH and NOT MATCH partitions must be disjoint"
    );
    assert_eq!(
        &matched | &unmatched,
        [1u64, 2, 3, 4].into_iter().collect(),
        "MATCH ∪ NOT MATCH must cover every stored fact"
    );
}

// ============================================================================
// 5. relate with properties -> property-filtered MATCH
// ============================================================================

/// GIVEN edges 1-[:SUPPORTS {weight: 0.9}]->2 and 2-[:SUPPORTS {weight: 0.4}]->3
/// WHEN a GraphFirst MATCH binds the edge alias `r` and filters `r.weight = 0.9`
/// THEN exactly the TARGET of the weight=0.9 edge is returned: fact 2 (edge
///      1->2). The weight=0.4 edge (2->3) is excluded by the edge-property
///      predicate — proving `r.weight` resolves against the EDGE, not a node.
#[test]
fn test_edge_property_filtered_match_selects_target_of_weight_0_9() {
    let (_dir, db, _memory, _e12) = setup_memory_graph();

    let results = run_match_on_semantic(
        &db,
        "MATCH (a)-[r:SUPPORTS]->(b) WHERE r.weight = 0.9 RETURN b LIMIT 20",
    );

    assert_eq!(
        result_ids(&results),
        [2u64].into_iter().collect(),
        "only the target of the weight=0.9 SUPPORTS edge (1->2) is fact 2"
    );
}

/// GIVEN the same weighted SUPPORTS edges (weights 0.9 and 0.4)
/// WHEN the GraphFirst MATCH filters the edge property `r.weight = 0.4`
/// THEN exactly the TARGET of the weight=0.4 edge is returned: fact 3 (edge
///      2->3). The weight=0.9 edge (1->2) is excluded.
#[test]
fn test_edge_property_filtered_match_selects_target_of_weight_0_4() {
    let (_dir, db, _memory, _e12) = setup_memory_graph();

    let results = run_match_on_semantic(
        &db,
        "MATCH (a)-[r:SUPPORTS]->(b) WHERE r.weight = 0.4 RETURN b LIMIT 20",
    );

    assert_eq!(
        result_ids(&results),
        [3u64].into_iter().collect(),
        "only the target of the weight=0.4 SUPPORTS edge (2->3) is fact 3"
    );
}

// ============================================================================
// 6. Deleting an endpoint cascades dangling edges out of relations()
// ============================================================================

/// GIVEN fact 1 with outgoing SUPPORTS->2 and CONTRADICTS->4
/// WHEN endpoint fact 2 is deleted
/// THEN the dangling SUPPORTS->2 edge is cascaded away and `relations(1)`
///      shrinks to exactly the CONTRADICTS->4 edge — no dangling edge survives.
#[test]
fn test_deleting_endpoint_cascades_dangling_edge_out_of_relations() {
    let (_dir, _db, memory, _e12) = setup_memory_graph();
    let sm = memory.semantic();

    sm.delete(2).expect("delete fact 2");

    let rels = sm.relations(1).expect("relations(1) after delete");
    assert_eq!(
        edge_targets(&rels),
        [(4, "CONTRADICTS".to_string())].into_iter().collect(),
        "deleting fact 2 must cascade the SUPPORTS->2 edge away"
    );
}

/// GIVEN fact 2 which is BOTH a SUPPORTS target (1->2) and source (2->3)
/// WHEN fact 2 is deleted
/// THEN MATCH (a)-[:SUPPORTS]->(b) collapses to the empty set: edge 1->2 lost
///      its target and edge 2->3 lost its source, so no SUPPORTS edge remains
///      with both endpoints live.
#[test]
fn test_match_supports_empty_after_pivotal_fact_deleted() {
    let (_dir, db, memory, _e12) = setup_memory_graph();

    memory.semantic().delete(2).expect("delete pivotal fact 2");

    let results = execute_sql(
        &db,
        "SELECT * FROM _semantic_memory WHERE MATCH (a)-[:SUPPORTS]->(b) LIMIT 20",
    )
    .expect("MATCH after delete must execute");

    assert!(
        result_ids(&results).is_empty(),
        "both SUPPORTS edges lost an endpoint; none can anchor a match"
    );
}
