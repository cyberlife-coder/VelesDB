//! BDD tests for the GraphFirst MATCH execution strategy (S4-17).
//!
//! The MATCH query planner (`MatchQueryPlanner::plan`) selects
//! `MatchExecutionStrategy::GraphFirst` when the WHERE clause contains
//! **no** `similarity()` predicate. GraphFirst traverses the graph from the
//! start node (optionally filtered by labels + property index), then
//! evaluates the remaining WHERE conditions. It is the default strategy for
//! pure-graph queries and queries whose WHERE clause only touches scalar
//! properties.
//!
//! These tests exercise the **full pipeline**: SQL string -> parse ->
//! planner strategy selection -> execute -> verify bindings.
//!
//! Two complementary assertion strategies are used:
//!
//! 1. **Planner-level**: call `MatchQueryPlanner::plan` directly on the
//!    parsed `MatchClause` with a representative `CollectionStats` to assert
//!    that `GraphFirst` is the strategy the planner would pick. This gives
//!    deterministic coverage of strategy selection without depending on
//!    execution side-effects.
//! 2. **Behavior-level**: execute the same query through
//!    `Database::execute_query` and verify the returned node bindings match
//!    what a correct GraphFirst execution must yield.
//!
//! Coverage breakdown (per `.claude/rules/bdd-testing.md`):
//!
//! | Category | Count | Share |
//! |----------|-------|-------|
//! | Nominal  |   3   |  50%  |
//! | Edge     |   2   |  33%  |
//! | Negative |   1   |  17%  |

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::collection::search::query::match_planner::{
    CollectionStats, MatchExecutionStrategy, MatchQueryPlanner,
};
use velesdb_core::{Database, GraphEdge, Point};

use super::helpers::create_test_db;

// =========================================================================
// Module-specific setup
// =========================================================================

/// Creates a `VectorCollection` with 5 labeled nodes and a single CITES
/// edge, yielding a lopsided graph so GraphFirst traversal is clearly
/// cheaper than vector scan.
///
/// Graph topology:
/// ```text
///   (1:Document {category:"science"})--[:CITES]-->(2:Reference {category:"science"})
///   (3:Document {category:"tech"})     (no outgoing edge)
///   (4:Document {category:"tech"})     (no outgoing edge)
///   (5:Reference {category:"history"}) (isolated)
/// ```
fn setup_graph_first_collection(db: &Database) {
    db.create_vector_collection("papers", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create papers collection");
    let vc = db
        .get_vector_collection("papers")
        .expect("test: get papers collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Document"],
                "title": "Physics 101",
                "category": "science"
            })),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({
                "_labels": ["Reference"],
                "title": "Newton's Laws",
                "category": "science"
            })),
        ),
        Point::new(
            3,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({
                "_labels": ["Document"],
                "title": "Rust Handbook",
                "category": "tech"
            })),
        ),
        Point::new(
            4,
            vec![0.05, 0.95, 0.0, 0.0],
            Some(json!({
                "_labels": ["Document"],
                "title": "Python Guide",
                "category": "tech"
            })),
        ),
        Point::new(
            5,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "_labels": ["Reference"],
                "title": "Ancient Rome",
                "category": "history"
            })),
        ),
    ])
    .expect("test: upsert corpus");

    let edge = GraphEdge::new(100, 1, 2, "CITES").expect("test: create edge 1->2");
    vc.add_edge(edge).expect("test: add edge 1->2 CITES");
}

/// Builds a params map with only the `_collection` routing key.
fn collection_param(collection: &str) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String(collection.to_string()),
    );
    params
}

/// Stats representative of the seeded collection: 5 nodes, 1 edge, 2 labels.
fn seeded_stats() -> CollectionStats {
    CollectionStats {
        total_nodes: 5,
        total_edges: 1,
        avg_degree: 0.2,
        label_count: 2,
        label_selectivity: 0.5,
    }
}

/// Parses `sql` and asserts that `MatchQueryPlanner::plan` selects
/// `GraphFirst` for the representative stats.
fn assert_graph_first_planned(sql: &str) {
    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");
    let match_clause = query
        .match_clause
        .as_ref()
        .expect("test: query must have a MATCH clause");
    let strategy = MatchQueryPlanner::plan(match_clause, &seeded_stats());
    assert!(
        matches!(strategy, MatchExecutionStrategy::GraphFirst { .. }),
        "planner must select GraphFirst for '{sql}', got: {strategy:?}"
    );
}

// =========================================================================
// A. Nominal tests (~50%)
// =========================================================================

/// GIVEN a collection with Documents linked to References via CITES edges
/// WHEN a MATCH query traverses `(doc:Document)-[:CITES]->(ref)` with NO
///      `similarity()` predicate
/// THEN the planner picks GraphFirst and execution returns the traversal
///      endpoint (ref = node 2) reached from the only Document that has
///      an outgoing CITES edge (doc = node 1).
///
/// Note: per the MATCH execution contract,
/// `SearchResult.point.id == traversal_result.target_id`, i.e. the terminal
/// node of each traversal branch. The start node is exposed via the
/// `bindings` map on `MatchResult`, not via `SearchResult.point.id`.
#[test]
fn test_match_graph_first_basic_traversal_returns_connected_pair() {
    let (_dir, db) = create_test_db();
    setup_graph_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) RETURN doc, ref LIMIT 10";
    assert_graph_first_planned(sql);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH");
    let params = collection_param("papers");
    let results = db
        .execute_query(&query, &params)
        .expect("test: GraphFirst MATCH should succeed");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    // The only CITES edge is 1 -> 2. Node 2 is the unique traversal target.
    assert_eq!(
        ids,
        vec![2],
        "only reachable target node is 2 (Reference via CITES), got: {ids:?}"
    );
    // Isolated Documents (3, 4) have no CITES edge and must NOT appear.
    assert!(
        !ids.contains(&3) && !ids.contains(&4),
        "isolated Documents 3 and 4 must not appear as traversal targets, got: {ids:?}"
    );
}

/// GIVEN the same graph
/// WHEN a MATCH query adds a scalar property predicate on the start node
///      (`WHERE doc.category = 'science'`) with NO similarity predicate
/// THEN the planner still picks GraphFirst (property index prefilter +
///      traversal). Only node 1 (science Document) satisfies the prefilter
///      and has an outgoing CITES edge, so node 2 appears as the unique
///      traversal target.
#[test]
fn test_match_graph_first_with_start_property_predicate() {
    let (_dir, db) = create_test_db();
    setup_graph_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE doc.category = 'science' \
               RETURN doc, ref LIMIT 10";
    assert_graph_first_planned(sql);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH");
    let params = collection_param("papers");
    let results = db
        .execute_query(&query, &params)
        .expect("test: GraphFirst with predicate should succeed");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    // Only doc=1 (science, has CITES edge) passes the predicate -> target = 2.
    assert_eq!(
        ids,
        vec![2],
        "only traversal target from a science Document is node 2, got: {ids:?}"
    );
    // Tech Documents (3, 4) have no CITES edge AND wrong category -- excluded.
    assert!(
        !ids.contains(&3) && !ids.contains(&4),
        "tech Documents must not appear as targets, got: {ids:?}"
    );
}

/// GIVEN the same graph
/// WHEN a MATCH query with no relationship pattern filters by label +
///      property (`MATCH (n:Document) WHERE n.category = 'tech'`)
/// THEN the planner picks GraphFirst (no similarity) and returns every
///      Document matching the property predicate (nodes 3 and 4).
#[test]
fn test_match_graph_first_label_only_pattern_returns_all_matches() {
    let (_dir, db) = create_test_db();
    setup_graph_first_collection(&db);

    let sql = "MATCH (n:Document) WHERE n.category = 'tech' RETURN n LIMIT 10";
    assert_graph_first_planned(sql);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH");
    let params = collection_param("papers");
    let results = db
        .execute_query(&query, &params)
        .expect("test: GraphFirst single-node MATCH should succeed");

    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    let expected: std::collections::HashSet<u64> = [3, 4].into_iter().collect();
    assert_eq!(
        ids, expected,
        "GraphFirst must return exactly the tech Documents"
    );
}

// =========================================================================
// B. Edge tests (~33%)
// =========================================================================

/// GIVEN the seeded graph (only one CITES edge exists: 1->2)
/// WHEN a MATCH query requests an unknown relationship type
///      `[:AUTHORED_BY]` that no edge uses
/// THEN the planner picks GraphFirst and execution returns an empty set
///      (no panic, no error).
#[test]
fn test_match_graph_first_no_matching_relationship_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_graph_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:AUTHORED_BY]->(a) RETURN doc, a LIMIT 10";
    assert_graph_first_planned(sql);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH");
    let params = collection_param("papers");
    let results = db
        .execute_query(&query, &params)
        .expect("test: unknown relationship should not error");

    assert!(
        results.is_empty(),
        "no edges match AUTHORED_BY, expected empty result, got {} rows",
        results.len()
    );
}

/// GIVEN a collection populated with nodes but zero edges
/// WHEN a MATCH query requires a relationship traversal
/// THEN the planner picks GraphFirst and execution returns an empty set.
#[test]
fn test_match_graph_first_empty_edge_store_returns_empty() {
    let (_dir, db) = create_test_db();
    db.create_vector_collection("isolates", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create isolates collection");
    let vc = db
        .get_vector_collection("isolates")
        .expect("test: get isolates");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"_labels": ["Document"], "title": "A"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"_labels": ["Document"], "title": "B"})),
        ),
    ])
    .expect("test: upsert isolates");

    let sql = "MATCH (d:Document)-[:CITES]->(r) RETURN d LIMIT 10";
    assert_graph_first_planned(sql);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH");
    let params = collection_param("isolates");
    let results = db
        .execute_query(&query, &params)
        .expect("test: traversal on isolate-only graph should not error");

    assert!(
        results.is_empty(),
        "no edges exist, expected empty result, got {} rows",
        results.len()
    );
}

// =========================================================================
// C. Negative tests (>= 17%)
// =========================================================================

/// GIVEN a MATCH query referencing a collection that does not exist
/// WHEN the query is executed
/// THEN an explicit error is returned (not a panic, not empty results).
#[test]
fn test_match_graph_first_missing_collection_errors() {
    let (_dir, db) = create_test_db();

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) RETURN doc LIMIT 10";
    // Planner selection is independent of collection existence -- still
    // GraphFirst because there is no similarity() predicate.
    assert_graph_first_planned(sql);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH");
    let params = collection_param("does_not_exist");

    let err = db
        .execute_query(&query, &params)
        .expect_err("test: missing collection must produce a clean error");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("not found") || msg.contains("collection") || msg.contains("does_not_exist"),
        "error should reference the missing collection, got: {err}"
    );
}
