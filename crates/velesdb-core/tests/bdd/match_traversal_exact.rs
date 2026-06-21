//! BDD tests pinning variable-length GRAPH TRAVERSAL result-sets EXACTLY.
//!
//! These tests fix the contract of var-length MATCH patterns
//! (`-[:REL*m..n]->`) against a single hand-computed graph, so any regression
//! in reachability, hop bounds, direction, edge-type union, edge-property
//! filtering, or negation is caught by an exact assertion.
//!
//! Two execution forms are exercised (both verified against the codebase):
//!
//! 1. Bare `MATCH (a:Start)-[...]->(c) RETURN c` routed via the `_collection`
//!    param. Per the MATCH execution contract,
//!    `SearchResult.point.id == traversal_result.target_id` (the TERMINAL node
//!    of each branch); the start node lives in the `_bindings` map.
//! 2. `SELECT * FROM <coll> AS <alias> WHERE NOT MATCH (ctx)-[:REL]->(x)` —
//!    NOT MATCH returns the FROM rows whose anchor has NO matching outgoing
//!    edge (the complement of the source set).
//!
//! ## The graph (one fixed topology for every test)
//!
//! ```text
//!   (1:Start) -[:KNOWS {w:10}]-> (2) -[:KNOWS {w:20}]-> (3) -[:KNOWS {w:30}]-> (4)
//!   (1:Start) -[:FRIEND {w:50}]-> (5)
//! ```
//!
//! Edge ids: 100 (1->2), 101 (2->3), 102 (3->4) all KNOWS; 103 (1->5) FRIEND.
//! Node 1 is the only `Start`-labeled node, so it is the unique anchor.
//!
//! KNOWS-reachability from node 1 by hop count:
//!   * hop 1: {2}      (1->2)
//!   * hop 2: {3}      (1->2->3)
//!   * hop 3: {4}      (1->2->3->4)
//! FRIEND-reachability from node 1: hop 1 = {5}.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point, SearchResult};

use super::helpers::{create_test_db, execute_sql, result_ids};

// =========================================================================
// Fixed topology + execution helpers
// =========================================================================

/// Builds the fixed `social` graph described in the module doc-comment.
fn setup_social_graph(db: &Database) {
    db.create_vector_collection("social", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create social collection");
    let vc = db
        .get_vector_collection("social")
        .expect("test: get social collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"_labels": ["Start"], "name": "A"})),
        ),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"name": "B"}))),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"name": "C"}))),
        Point::new(4, vec![0.0, 0.0, 0.0, 1.0], Some(json!({"name": "D"}))),
        Point::new(5, vec![1.0, 1.0, 0.0, 0.0], Some(json!({"name": "E"}))),
    ])
    .expect("test: upsert social nodes");

    for (edge_id, src, dst, label, w) in [
        (100u64, 1u64, 2u64, "KNOWS", 10),
        (101, 2, 3, "KNOWS", 20),
        (102, 3, 4, "KNOWS", 30),
        (103, 1, 5, "FRIEND", 50),
    ] {
        let mut props = HashMap::new();
        props.insert("w".to_string(), json!(w));
        let edge = GraphEdge::new(edge_id, src, dst, label)
            .expect("test: create social edge")
            .with_properties(props);
        vc.add_edge(edge).expect("test: add social edge");
    }
}

/// Routes a bare-MATCH query to the `social` collection.
fn social_param() -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert("_collection".to_string(), json!("social"));
    params
}

/// Parses and executes a bare `MATCH ... RETURN` query against `social`.
fn run(db: &Database, sql: &str) -> Vec<SearchResult> {
    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");
    db.execute_query(&query, &social_param())
        .expect("test: execute MATCH query")
}

// =========================================================================
// 1..7. Exact var-length traversal result-sets
// =========================================================================

/// GIVEN the fixed graph
/// WHEN matching `(a:Start)-[:KNOWS*1..1]->(c)` from the start anchor (node 1)
/// THEN the terminal-node set is exactly {2}: node 1 has one KNOWS edge
///      (1->2), so the only 1-hop KNOWS target is node 2.
#[test]
fn test_var_length_one_hop_exact_set() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = run(&db, "MATCH (a:Start)-[:KNOWS*1..1]->(c) RETURN c LIMIT 10");

    assert_eq!(
        result_ids(&results),
        [2u64].into_iter().collect(),
        "the only 1-hop KNOWS target from node 1 is node 2"
    );
}

/// GIVEN the fixed graph
/// WHEN matching `(a:Start)-[:KNOWS*2..2]->(c)`
/// THEN the terminal-node set is exactly {3}: the unique 2-hop KNOWS path is
///      1->2->3, so node 3 is the only target (node 2 at hop 1 is excluded).
#[test]
fn test_var_length_two_hop_exact_set() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = run(&db, "MATCH (a:Start)-[:KNOWS*2..2]->(c) RETURN c LIMIT 10");

    assert_eq!(
        result_ids(&results),
        [3u64].into_iter().collect(),
        "the only 2-hop KNOWS target from node 1 is node 3 (via 1->2->3)"
    );
}

/// GIVEN the fixed graph
/// WHEN matching `(a:Start)-[:KNOWS*1..3]->(c)`
/// THEN the terminal-node set is the union over hops 1, 2 and 3:
///      {2} ∪ {3} ∪ {4} = {2, 3, 4}. FRIEND target node 5 is excluded
///      (wrong edge type).
#[test]
fn test_var_length_one_to_three_hops_exact_union() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = run(&db, "MATCH (a:Start)-[:KNOWS*1..3]->(c) RETURN c LIMIT 10");

    assert_eq!(
        result_ids(&results),
        [2u64, 3, 4].into_iter().collect(),
        "1..3 hop KNOWS reach is {{2,3,4}}; node 5 (FRIEND) is excluded"
    );
}

/// GIVEN the fixed graph
/// WHEN matching the UNDIRECTED `(a:Start)-[:KNOWS*2..2]-(c)`
/// THEN the terminal-node set is exactly {1, 3}. From node 1, undirected
///      2-hop KNOWS walks (no edge reused) are 1-2-3 (target 3) and the
///      back-and-forth 1-2-1 is forbidden by relationship isomorphism; the
///      only other 2-hop walk that returns to 1 would need a second distinct
///      edge incident to node 2, which does not exist. With a directed graph
///      treated undirected, node 1 reaches node 3 forward (1->2->3). Node 1
///      itself is NOT a valid target (would reuse edge 100). So target = {3}.
#[test]
fn test_var_length_undirected_two_hop_exact_set() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = run(&db, "MATCH (a:Start)-[:KNOWS*2..2]-(c) RETURN c LIMIT 10");

    assert_eq!(
        result_ids(&results),
        [3u64].into_iter().collect(),
        "undirected 2-hop KNOWS from node 1 reaches only node 3 (1-2-3); \
         reusing edge 100 to bounce back to node 1 is forbidden"
    );
}

/// GIVEN the fixed graph
/// WHEN matching `(a:Start)-[r:KNOWS*1..3]->(c) WHERE r.w = 20`
/// THEN ANY-element semantics keep only paths where some traversed edge has
///      w=20. Edge 2->3 (id 101) carries w=20. The shortest path containing
///      it is 1->2->3 (target 3); the 3-hop path 1->2->3->4 (target 4) also
///      contains it. So the exact target set is {3, 4}.
#[test]
fn test_var_length_edge_property_filter_matches() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = run(
        &db,
        "MATCH (a:Start)-[r:KNOWS*1..3]->(c) WHERE r.w = 20 RETURN c LIMIT 10",
    );

    assert_eq!(
        result_ids(&results),
        [3u64, 4].into_iter().collect(),
        "paths containing the w=20 edge (2->3) terminate at nodes 3 and 4"
    );
}

/// GIVEN the fixed graph
/// WHEN matching `(a:Start)-[r:KNOWS*1..3]->(c) WHERE r.w = 999`
/// THEN no traversed edge carries w=999, so ANY-element semantics reject
///      every path: the result is empty.
#[test]
fn test_var_length_edge_property_filter_empty() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = run(
        &db,
        "MATCH (a:Start)-[r:KNOWS*1..3]->(c) WHERE r.w = 999 RETURN c LIMIT 10",
    );

    assert!(
        results.is_empty(),
        "no KNOWS edge carries w=999, so every path is rejected, got {} row(s)",
        results.len()
    );
}

/// GIVEN the fixed graph
/// WHEN matching the multi-type single-hop `(a:Start)-[:KNOWS|FRIEND*1..1]->(c)`
/// THEN the union of edge types surfaces BOTH the KNOWS edge (1->2) and the
///      FRIEND edge (1->5). Exact target set = {2, 5}.
#[test]
fn test_var_length_multi_type_union_exact_set() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = run(
        &db,
        "MATCH (a:Start)-[:KNOWS|FRIEND*1..1]->(c) RETURN c LIMIT 10",
    );

    assert_eq!(
        result_ids(&results),
        [2u64, 5].into_iter().collect(),
        "KNOWS|FRIEND 1-hop from node 1 reaches node 2 (KNOWS) and node 5 (FRIEND)"
    );
}

/// GIVEN the fixed graph
/// WHEN excluding the pattern with `NOT MATCH (ctx)-[:KNOWS]->(x)` over all
///      `social` rows (implicit anchor binds `ctx` to the FROM rows)
/// THEN only nodes WITHOUT an outgoing KNOWS edge survive. Outgoing KNOWS
///      sources are {1, 2, 3}; the complement over {1,2,3,4,5} is {4, 5}.
#[test]
fn test_not_match_outgoing_knows_complement_set() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM social AS node \
         WHERE NOT MATCH (ctx)-[:KNOWS]->(x) LIMIT 20",
    )
    .expect("NOT MATCH with implicit anchor must execute");

    assert_eq!(
        result_ids(&results),
        [4u64, 5].into_iter().collect(),
        "nodes 4 and 5 are the only ones with NO outgoing KNOWS edge"
    );
}

/// GIVEN the fixed graph
/// WHEN excluding `NOT MATCH (ctx)-[:FRIEND]->(x)` over all `social` rows
/// THEN only nodes WITHOUT an outgoing FRIEND edge survive. The single FRIEND
///      source is node 1; the complement over {1,2,3,4,5} is {2, 3, 4, 5}.
#[test]
fn test_not_match_outgoing_friend_complement_set() {
    let (_dir, db) = create_test_db();
    setup_social_graph(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM social AS node \
         WHERE NOT MATCH (ctx)-[:FRIEND]->(x) LIMIT 20",
    )
    .expect("NOT MATCH with implicit anchor must execute");

    assert_eq!(
        result_ids(&results),
        [2u64, 3, 4, 5].into_iter().collect(),
        "node 1 is the only FRIEND source; its complement is {{2,3,4,5}}"
    );
}
