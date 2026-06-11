//! BDD tests for MATCH relationship semantics (audit 2026-06 finding F).
//!
//! Covers two correctness contracts of the MATCH pattern walker:
//!
//! 1. **Relationship isomorphism** (Cypher semantics): a single edge may be
//!    traversed at most once per matched path. Without it, undirected
//!    (`Both`) and cyclic variable-length patterns fabricate phantom paths
//!    by bouncing back and forth on the same edge.
//! 2. **Relationship alias binding**: `-[r:KNOWS]->` must bind `r` to the
//!    traversed edge so `WHERE r.prop` and `RETURN r.prop` resolve against
//!    the EDGE's properties, not the target node's payload.
//!
//! All tests exercise the full pipeline: SQL string -> parse -> execute.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point, SearchResult};

use super::helpers::create_test_db;

// =========================================================================
// Module-specific setup
// =========================================================================

/// Builds a params map with only the `_collection` routing key.
fn match_collection_param(collection: &str) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String(collection.to_string()),
    );
    params
}

/// Parses `sql` and executes it against `collection`.
fn run_match(db: &Database, sql: &str, collection: &str) -> Vec<SearchResult> {
    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");
    db.execute_query(&query, &match_collection_param(collection))
        .expect("test: execute MATCH query")
}

/// Parses `sql` and executes it with a `$q` vector parameter bound.
fn run_match_with_vector(
    db: &Database,
    sql: &str,
    collection: &str,
    vector: &[f32],
) -> Vec<SearchResult> {
    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse hybrid MATCH query");
    let mut params = match_collection_param(collection);
    params.insert("q".to_string(), json!(vector));
    db.execute_query(&query, &params)
        .expect("test: execute hybrid MATCH query")
}

/// GIVEN base: two nodes and a single `KNOWS` edge 1 -> 2 carrying
/// `{since: 2020}`.
fn setup_single_edge_collection(db: &Database) {
    db.create_vector_collection("pair", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create pair collection");
    let vc = db
        .get_vector_collection("pair")
        .expect("test: get pair collection");

    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"name": "A"}))),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"name": "B"}))),
    ])
    .expect("test: upsert nodes");

    let mut props = HashMap::new();
    props.insert("since".to_string(), json!(2020));
    let edge = GraphEdge::new(100, 1, 2, "KNOWS")
        .expect("test: create edge 1->2")
        .with_properties(props);
    vc.add_edge(edge).expect("test: add edge 1->2 KNOWS");
}

/// GIVEN base: a directed triangle 1 -> 2 -> 3 -> 1 (all `KNOWS`), with
/// node 1 labeled `Start` so it is the only traversal anchor.
fn setup_triangle_collection(db: &Database) {
    db.create_vector_collection("triangle", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create triangle collection");
    let vc = db
        .get_vector_collection("triangle")
        .expect("test: get triangle collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"_labels": ["Start"], "name": "A"})),
        ),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"name": "B"}))),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"name": "C"}))),
    ])
    .expect("test: upsert nodes");

    for (id, source, target) in [(101, 1, 2), (102, 2, 3), (103, 3, 1)] {
        let edge = GraphEdge::new(id, source, target, "KNOWS").expect("test: create edge");
        vc.add_edge(edge).expect("test: add edge");
    }
}

// =========================================================================
// A. Relationship isomorphism (Bug 1)
// =========================================================================

/// GIVEN a graph with a single edge 1-[KNOWS]->2
/// WHEN matching the undirected variable-length pattern `(a)-[:KNOWS*2..2]-(b)`
/// THEN no path exists: a 2-hop path would have to traverse the only edge
///      twice (forward then backward), which relationship isomorphism forbids.
#[test]
fn test_match_undirected_two_hops_cannot_reuse_single_edge() {
    let (_dir, db) = create_test_db();
    setup_single_edge_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a)-[:KNOWS*2..2]-(b) RETURN a, b LIMIT 10",
        "pair",
    );

    assert!(
        results.is_empty(),
        "an edge must be traversed at most once per path; \
         bouncing 1-2-1 on the same edge fabricated {} phantom result(s)",
        results.len()
    );
}

/// GIVEN a directed triangle 1->2->3->1 (KNOWS)
/// WHEN matching `(a:Start)-[:KNOWS*3..3]->(b)`
/// THEN exactly one cycle is found (1->2->3->1, three distinct edges) and
///      it terminates back on the start node.
#[test]
fn test_match_triangle_cycle_found_without_edge_reuse() {
    let (_dir, db) = create_test_db();
    setup_triangle_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a:Start)-[:KNOWS*3..3]->(b) RETURN a, b LIMIT 10",
        "triangle",
    );

    assert_eq!(
        results.len(),
        1,
        "the triangle has exactly one 3-hop cycle from node 1"
    );
    assert_eq!(
        results[0].point.id, 1,
        "the 3-hop cycle must terminate back on the start node"
    );
}

/// GIVEN a directed triangle 1->2->3->1 (KNOWS)
/// WHEN matching `(a:Start)-[:KNOWS*4..4]->(b)`
/// THEN no path exists: a 4th hop would have to reuse an already-traversed
///      edge (1->2->3->1->2 reuses edge 1->2).
#[test]
fn test_match_four_hops_on_triangle_requires_distinct_edges() {
    let (_dir, db) = create_test_db();
    setup_triangle_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a:Start)-[:KNOWS*4..4]->(b) RETURN a, b LIMIT 10",
        "triangle",
    );

    assert!(
        results.is_empty(),
        "4 hops on a 3-edge triangle must reuse an edge; \
         relationship isomorphism forbids it, got {} result(s)",
        results.len()
    );
}

// =========================================================================
// B. Relationship alias binding (Bug 2)
// =========================================================================

/// GIVEN an edge 1-[KNOWS {since: 2020}]->2
/// WHEN filtering with `WHERE r.since = 2020`
/// THEN the pattern matches: `r` resolves to the traversed edge's properties.
#[test]
fn test_match_where_edge_property_matches() {
    let (_dir, db) = create_test_db();
    setup_single_edge_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) WHERE r.since = 2020 RETURN a, b LIMIT 10",
        "pair",
    );

    assert_eq!(
        results.len(),
        1,
        "WHERE r.since = 2020 must match the edge property"
    );
    assert_eq!(results[0].point.id, 2, "traversal target must be node 2");
}

/// GIVEN an edge 1-[KNOWS {since: 2020}]->2
/// WHEN filtering with `WHERE r.since = 1999`
/// THEN the pattern does not match.
#[test]
fn test_match_where_edge_property_rejects_non_matching() {
    let (_dir, db) = create_test_db();
    setup_single_edge_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) WHERE r.since = 1999 RETURN a, b LIMIT 10",
        "pair",
    );

    assert!(
        results.is_empty(),
        "WHERE r.since = 1999 must not match an edge with since = 2020"
    );
}

/// GIVEN an edge 1-[KNOWS {since: 2020}]->2
/// WHEN filtering with `WHERE r.since IN (2019, 2020)` (metadata condition)
/// THEN the pattern matches via the filter-engine path as well.
#[test]
fn test_match_where_edge_property_in_list() {
    let (_dir, db) = create_test_db();
    setup_single_edge_collection(&db);

    let hit = run_match(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) WHERE r.since IN (2019, 2020) RETURN a, b LIMIT 10",
        "pair",
    );
    assert_eq!(hit.len(), 1, "r.since IN (2019, 2020) must match the edge");

    let miss = run_match(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) WHERE r.since IN (1998, 1999) RETURN a, b LIMIT 10",
        "pair",
    );
    assert!(
        miss.is_empty(),
        "r.since IN (1998, 1999) must not match an edge with since = 2020"
    );
}

// =========================================================================
// C. Hybrid: similarity() on the start alias + relationship alias
//    (audit 2026-06 cluster F2 — plan-dependent edge-alias semantics)
//
//    `similarity()` on the START alias routes the planner toward
//    VectorFirst, which does not bind relationship aliases. Whenever the
//    WHERE or RETURN clause references a relationship alias, the planner
//    must fall back to GraphFirst so `r.prop` resolves against the EDGE.
// =========================================================================

/// GIVEN an edge 1-[KNOWS {since: 2020}]->2 and node 1 aligned with `$q`
/// WHEN filtering with `similarity(a.embedding, $q) > 0.1 AND r.since = 2020`
/// THEN the row is returned and `r.since` projects the edge property —
///      identical to the same query without the similarity predicate.
#[test]
fn test_match_similarity_with_edge_property_filter_matches() {
    let (_dir, db) = create_test_db();
    setup_single_edge_collection(&db);

    let results = run_match_with_vector(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) \
         WHERE similarity(a.embedding, $q) > 0.1 AND r.since = 2020 \
         RETURN a, r.since LIMIT 10",
        "pair",
        &[1.0, 0.0, 0.0, 0.0],
    );

    assert_eq!(
        results.len(),
        1,
        "similarity(a) AND r.since = 2020 must match the single edge; \
         the plan must not change edge-alias semantics"
    );
    let projected = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("r.since"))
        .and_then(serde_json::Value::as_i64);
    assert_eq!(
        projected,
        Some(2020),
        "RETURN r.since must project the edge property under the hybrid plan, \
         got payload: {:?}",
        results[0].point.payload
    );
}

/// GIVEN an edge 1-[KNOWS {since: 2020}]->2 and node 1 aligned with `$q`
/// WHEN filtering with `similarity(a.embedding, $q) > 0.1 AND r.since = 1999`
/// THEN no row is returned (the edge property does not match).
#[test]
fn test_match_similarity_with_edge_property_filter_rejects_non_matching() {
    let (_dir, db) = create_test_db();
    setup_single_edge_collection(&db);

    let results = run_match_with_vector(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) \
         WHERE similarity(a.embedding, $q) > 0.1 AND r.since = 1999 \
         RETURN a, r.since LIMIT 10",
        "pair",
        &[1.0, 0.0, 0.0, 0.0],
    );

    assert!(
        results.is_empty(),
        "r.since = 1999 must not match an edge with since = 2020, got {} row(s)",
        results.len()
    );
}

/// GIVEN an edge 1-[KNOWS {since: 2020}]->2 and node 1 aligned with `$q`
/// WHEN filtering with `similarity(a.embedding, $q) > 0.1 AND r.since IS NULL`
/// THEN no row is returned: the edge HAS a `since` property. Evaluating
///      `r.since IS NULL` against a node payload (where the key is absent)
///      would fabricate a false positive.
#[test]
fn test_match_similarity_with_edge_is_null_rejects_existing_property() {
    let (_dir, db) = create_test_db();
    setup_single_edge_collection(&db);

    let results = run_match_with_vector(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) \
         WHERE similarity(a.embedding, $q) > 0.1 AND r.since IS NULL \
         RETURN a, b LIMIT 10",
        "pair",
        &[1.0, 0.0, 0.0, 0.0],
    );

    assert!(
        results.is_empty(),
        "r.since IS NULL must be false for an edge carrying since = 2020, \
         got {} false-positive row(s)",
        results.len()
    );
}

/// GIVEN an edge 1-[KNOWS {since: 2020}]->2
/// WHEN projecting `RETURN r.since`
/// THEN the projected value is the edge property 2020.
#[test]
fn test_match_return_edge_property() {
    let (_dir, db) = create_test_db();
    setup_single_edge_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) RETURN r.since LIMIT 10",
        "pair",
    );

    assert_eq!(results.len(), 1, "the single KNOWS edge must match");
    let projected = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("r.since"))
        .and_then(serde_json::Value::as_i64);
    assert_eq!(
        projected,
        Some(2020),
        "RETURN r.since must project the edge property, got payload: {:?}",
        results[0].point.payload
    );
}

// =========================================================================
// Parallel edges: edge-binding-aware dedup (audit 2026-06 follow-up)
// =========================================================================

/// GIVEN base: two nodes with TWO parallel `KNOWS` edges 1 -> 2 carrying
/// different `since` properties.
fn setup_parallel_edges_collection(db: &Database) {
    db.create_vector_collection("parallel", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create parallel collection");
    let vc = db
        .get_vector_collection("parallel")
        .expect("test: get parallel collection");

    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"name": "A"}))),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"name": "B"}))),
    ])
    .expect("test: upsert nodes");

    for (edge_id, since) in [(100u64, 2020), (101u64, 2024)] {
        let mut props = HashMap::new();
        props.insert("since".to_string(), json!(since));
        let edge = GraphEdge::new(edge_id, 1, 2, "KNOWS")
            .expect("test: create parallel edge")
            .with_properties(props);
        vc.add_edge(edge).expect("test: add parallel edge");
    }
}

/// WHEN two parallel aliased edges connect the same node pair
/// THEN MATCH returns one row per edge (not one collapsed row).
#[test]
fn test_parallel_edges_yield_one_row_per_edge() {
    let (_dir, db) = create_test_db();
    setup_parallel_edges_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) RETURN r.since LIMIT 10",
        "parallel",
    );

    assert_eq!(
        results.len(),
        2,
        "two parallel KNOWS edges must yield two rows"
    );
    let mut sinces: Vec<i64> = results
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("r.since"))
                .and_then(serde_json::Value::as_i64)
        })
        .collect();
    sinces.sort_unstable();
    assert_eq!(
        sinces,
        vec![2020, 2024],
        "each row must project its own edge's property"
    );
}

/// WHEN a parallel edge is filtered by an edge property
/// THEN only the matching edge's row survives.
#[test]
fn test_parallel_edges_where_filters_per_edge() {
    let (_dir, db) = create_test_db();
    setup_parallel_edges_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a)-[r:KNOWS]->(b) WHERE r.since >= 2024 RETURN r.since LIMIT 10",
        "parallel",
    );

    assert_eq!(results.len(), 1, "only the 2024 edge passes the filter");
}

// =========================================================================
// Variable-length relationship aliases: list semantics (openCypher)
// =========================================================================

/// GIVEN base: a chain 1 -> 2 -> 3 (both `KNOWS`), node 1 labeled `Start`,
/// edges carrying `{w: 10}` and `{w: 20}`.
fn setup_chain_collection(db: &Database) {
    db.create_vector_collection("chain", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create chain collection");
    let vc = db
        .get_vector_collection("chain")
        .expect("test: get chain collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"_labels": ["Start"], "name": "A"})),
        ),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"name": "B"}))),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"name": "C"}))),
    ])
    .expect("test: upsert chain nodes");

    for (edge_id, src, dst, w) in [(100u64, 1u64, 2u64, 10), (101, 2, 3, 20)] {
        let mut props = HashMap::new();
        props.insert("w".to_string(), json!(w));
        let edge = GraphEdge::new(edge_id, src, dst, "KNOWS")
            .expect("test: create chain edge")
            .with_properties(props);
        vc.add_edge(edge).expect("test: add chain edge");
    }
}

/// WHEN a variable-length alias is projected bare
/// THEN it binds the LIST of traversed edge ids (openCypher list semantics).
#[test]
fn test_var_length_alias_projects_edge_id_list() {
    let (_dir, db) = create_test_db();
    setup_chain_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a:Start)-[r:KNOWS*2..2]->(c) RETURN r LIMIT 10",
        "chain",
    );

    assert_eq!(results.len(), 1, "exactly one 2-hop path exists");
    let projected = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("r"))
        .cloned();
    assert_eq!(
        projected,
        Some(json!([100, 101])),
        "RETURN r on a var-length alias must project the ordered edge-id list"
    );
}

/// WHEN a property is projected through a variable-length alias
/// THEN the projection is the positional list of per-edge values.
#[test]
fn test_var_length_alias_projects_property_list() {
    let (_dir, db) = create_test_db();
    setup_chain_collection(&db);

    let results = run_match(
        &db,
        "MATCH (a:Start)-[r:KNOWS*2..2]->(c) RETURN r.w LIMIT 10",
        "chain",
    );

    assert_eq!(results.len(), 1);
    let projected = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("r.w"))
        .cloned();
    assert_eq!(
        projected,
        Some(json!([10, 20])),
        "RETURN r.w on a var-length alias must project the per-edge value list"
    );
}

/// WHEN a WHERE references a var-length alias property
/// THEN ANY-element semantics apply: one matching edge keeps the row.
#[test]
fn test_var_length_alias_where_uses_any_semantics() {
    let (_dir, db) = create_test_db();
    setup_chain_collection(&db);

    let any_hit = run_match(
        &db,
        "MATCH (a:Start)-[r:KNOWS*2..2]->(c) WHERE r.w = 20 RETURN c LIMIT 10",
        "chain",
    );
    assert_eq!(
        any_hit.len(),
        1,
        "one traversed edge has w=20, so the path must match"
    );

    let no_hit = run_match(
        &db,
        "MATCH (a:Start)-[r:KNOWS*2..2]->(c) WHERE r.w = 99 RETURN c LIMIT 10",
        "chain",
    );
    assert!(
        no_hit.is_empty(),
        "no traversed edge has w=99, so the path must not match"
    );
}

/// WHEN distinct paths reach the same target through different edge lists
/// THEN each path is a distinct row (the dedup key includes the edge list).
#[test]
fn test_var_length_distinct_paths_are_distinct_rows() {
    let (_dir, db) = create_test_db();
    setup_chain_collection(&db);
    let vc = db
        .get_vector_collection("chain")
        .expect("test: get chain collection");
    // Second 1 -> 2 edge: now TWO 2-hop paths reach node 3.
    let mut props = HashMap::new();
    props.insert("w".to_string(), json!(11));
    let edge = GraphEdge::new(102, 1, 2, "KNOWS")
        .expect("test: create second 1->2 edge")
        .with_properties(props);
    vc.add_edge(edge).expect("test: add second 1->2 edge");

    let results = run_match(
        &db,
        "MATCH (a:Start)-[r:KNOWS*2..2]->(c) RETURN r LIMIT 10",
        "chain",
    );

    assert_eq!(
        results.len(),
        2,
        "two distinct 2-hop edge paths must yield two rows"
    );
}
