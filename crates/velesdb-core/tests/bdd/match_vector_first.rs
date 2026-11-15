//! BDD tests for the VectorFirst MATCH execution strategy.
//!
//! When a MATCH query includes `similarity(field, $vector) > threshold` in
//! its WHERE clause and the similarity targets the start node, the MATCH
//! planner selects the VectorFirst strategy: vector search first to find
//! top-k candidates, then validate graph pattern existence for each.
//!
//! These tests exercise the **full pipeline**: SQL string -> parse ->
//! planner strategy selection -> execute -> verify results.
//!
//! Coverage breakdown (per `bdd-testing.md`):
//!
//! | Category | Count | Share |
//! |----------|-------|-------|
//! | Nominal  |   3   |  50%  |
//! | Edge     |   2   |  33%  |
//! | Negative |   1   |  17%  |

use std::collections::{HashMap, HashSet};

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point, SearchResult};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Creates a vector collection with labeled nodes and graph edges suitable
/// for VectorFirst MATCH queries.
///
/// Graph topology:
/// ```text
///   (1:Document)--[:CITES]-->(2:Reference)
///   (3:Document)--[:CITES]-->(4:Reference)
///   (5:Document)  (no outgoing CITES edge)
/// ```
///
/// Vectors (4-dim, cosine):
/// - id 1: `[1.0, 0.0, 0.0, 0.0]` — Document "Physics 101"
/// - id 2: `[0.9, 0.1, 0.0, 0.0]` — Reference "Newton's Laws"
/// - id 3: `[0.05, 0.95, 0.0, 0.0]` — Document "Rust Handbook" (far from query)
/// - id 4: `[0.0, 0.0, 1.0, 0.0]` — Reference "Ownership Model"
/// - id 5: `[0.95, 0.05, 0.0, 0.0]` — Document "Chemistry" (high sim, no edge)
fn setup_vector_first_collection(db: &Database) {
    // Create a vector collection (supports both vectors and graph edges).
    let vc = {
        db.create_vector_collection("papers", 4, velesdb_core::DistanceMetric::Cosine)
            .expect("test: create papers collection");
        db.get_vector_collection("papers")
            .expect("test: get papers collection")
    };

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
            vec![0.05, 0.95, 0.0, 0.0],
            Some(json!({
                "_labels": ["Document"],
                "title": "Rust Handbook",
                "category": "tech"
            })),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "_labels": ["Reference"],
                "title": "Ownership Model",
                "category": "tech"
            })),
        ),
        Point::new(
            5,
            vec![0.95, 0.05, 0.0, 0.0],
            Some(json!({
                "_labels": ["Document"],
                "title": "Chemistry Basics",
                "category": "science"
            })),
        ),
    ])
    .expect("test: upsert papers corpus");

    // Add graph edges: Document -> Reference via CITES.
    let edge1 = GraphEdge::new(100, 1, 2, "CITES").expect("test: create edge 1->2");
    vc.add_edge(edge1).expect("test: add edge 1->2 CITES");

    let edge2 = GraphEdge::new(101, 3, 4, "CITES").expect("test: create edge 3->4");
    vc.add_edge(edge2).expect("test: add edge 3->4 CITES");
}

/// The `doc` binding of each row: the Document a row starts from. A row's
/// point id is the walk's target (the cited node), as for any MATCH.
fn doc_bindings(results: &[velesdb_core::SearchResult]) -> Vec<u64> {
    results
        .iter()
        .map(|r| {
            r.point.payload.as_ref().expect("test: payload")["_bindings"]["doc"]
                .as_u64()
                .expect("test: doc binding")
        })
        .collect()
}

/// Builds a params map with `_collection` and a vector parameter.
fn match_params(
    collection: &str,
    param_name: &str,
    vector: &[f32],
) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String(collection.to_string()),
    );
    params.insert(param_name.to_string(), json!(vector));
    params
}

/// Builds a params map with only `_collection` (no vector param).
fn collection_param(collection: &str) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert(
        "_collection".to_string(),
        serde_json::Value::String(collection.to_string()),
    );
    params
}

// =========================================================================
// A. Nominal tests (~60%)
// =========================================================================

/// GIVEN a collection with Documents linked to References via CITES edges
/// WHEN a MATCH query uses `similarity(doc.embedding, $v) > 0.7` targeting
///      the start node (doc), combined with a `[:CITES]` relationship
/// THEN results include only nodes that pass BOTH similarity AND graph checks.
///
/// Here, node 1 (Physics 101, cosine ~1.0 to query) has a CITES edge to node 2,
/// so it should appear. Node 5 (Chemistry Basics, cosine ~0.999) does NOT have
/// a CITES edge, so VectorFirst must exclude it despite high similarity.
#[test]
fn test_match_vector_first_basic() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.7 \
               RETURN doc, ref LIMIT 10";
    let params = match_params("papers", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query =
        velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH with similarity");

    let results = db
        .execute_query(&query, &params)
        .expect("test: execute VectorFirst MATCH query");

    // Node 1 (Physics 101) should appear: high similarity AND has CITES edge.
    let ids = doc_bindings(&results);
    assert!(
        ids.contains(&1),
        "Node 1 (Physics 101) should pass both similarity and graph filter, got: {ids:?}"
    );
    assert!(
        !ids.contains(&5),
        "Node 5 (Chemistry Basics) has no CITES edge and must be excluded despite high similarity, got: {ids:?}"
    );
}

/// GIVEN the same graph topology
/// WHEN a VectorFirst query matches a Document that has a CITES edge to a Reference
/// THEN the result excludes Documents without CITES edges even if they have high
///      vector similarity.
///
/// Node 5 (Chemistry Basics, vector ~`[0.95, 0.05, 0, 0]`) has cosine > 0.7
/// to `[1, 0, 0, 0]` but no outgoing CITES edge, so it must be filtered out.
#[test]
fn test_match_vector_first_filters_by_graph_pattern() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.5 \
               RETURN doc LIMIT 20";
    let params = match_params("papers", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");

    let results = db
        .execute_query(&query, &params)
        .expect("test: execute query");

    let ids = doc_bindings(&results);

    // Node 5 has high similarity but no CITES edge -- must NOT appear.
    assert!(
        !ids.contains(&5),
        "Node 5 (Chemistry Basics) has no CITES edge and must be excluded, got: {ids:?}"
    );

    // Node 1 should be present (high sim + has CITES edge).
    assert!(
        ids.contains(&1),
        "Node 1 should pass both filters, got: {ids:?}"
    );
}

/// GIVEN the graph topology with multiple Documents at varying distances
/// WHEN VectorFirst runs with a low threshold (>0.0) and a relationship filter
/// THEN all Documents with matching graph pattern are returned, ordered by score.
#[test]
fn test_match_vector_first_returns_multiple_candidates() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.0 \
               RETURN doc LIMIT 20";
    let params = match_params("papers", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");

    let results = db
        .execute_query(&query, &params)
        .expect("test: execute query");

    // Both node 1 (Physics 101) and node 3 (Rust Handbook) have CITES edges.
    // Node 1 has high similarity, node 3 has low similarity, but threshold is >0.0.
    let ids = doc_bindings(&results);
    assert!(
        ids.contains(&1),
        "Node 1 (high sim + CITES edge) should appear, got: {ids:?}"
    );
    // The load-bearing check: node 3 has very low similarity (~0.05) so it ONLY
    // survives at threshold > 0.0; it must still be admitted because it has a
    // CITES edge. This proves the graph-validation path runs on a node the vector
    // filter does not carry on similarity alone (defeats a passthrough mutant).
    assert!(
        ids.contains(&3),
        "Node 3 (low sim + CITES edge) should appear at threshold >0.0, got: {ids:?}"
    );
    // Node 5 has high similarity but no CITES edge: graph filter must exclude it.
    assert!(
        !ids.contains(&5),
        "Node 5 has no CITES edge and must be excluded, got: {ids:?}"
    );
    // Docstring claims results are ordered by score; verify it (exercises sort_by_score).
    let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    assert!(
        scores.windows(2).all(|w| w[0] >= w[1]),
        "Results must be ordered by descending score, got: {scores:?}"
    );
}

// =========================================================================
// B. Edge tests (~20%)
// =========================================================================

/// GIVEN an empty collection with no points or edges
/// WHEN a VectorFirst MATCH query is executed
/// THEN the result is empty (not an error).
#[test]
fn test_match_vector_first_empty_collection() {
    let (_dir, db) = create_test_db();

    db.create_vector_collection("empty", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create empty collection");

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.5 \
               RETURN doc LIMIT 10";
    let params = match_params("empty", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query = velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query");

    let results = db
        .execute_query(&query, &params)
        .expect("test: VectorFirst on empty collection should not error");

    assert!(
        results.is_empty(),
        "Empty collection should return 0 results, got {}",
        results.len()
    );
}

/// GIVEN a populated collection
/// WHEN the similarity threshold is extremely high (>0.999)
/// THEN only near-exact matches with valid graph paths are returned.
///
/// Only node 1 (exact `[1,0,0,0]`) should pass `> 0.999` threshold, and it
/// has a CITES edge. Node 5 (`[0.95, 0.05, 0, 0]`) has cosine ~0.999 but
/// may or may not pass depending on precision; regardless, it has no CITES
/// edge and should be excluded by the graph filter.
#[test]
fn test_match_vector_first_high_threshold() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.999 \
               RETURN doc LIMIT 10";
    let params = match_params("papers", "v", &[1.0, 0.0, 0.0, 0.0]);

    let query =
        velesdb_core::velesql::Parser::parse(sql).expect("test: parse high-threshold MATCH query");

    let results = db
        .execute_query(&query, &params)
        .expect("test: execute high-threshold query");

    // With threshold > 0.999 and cosine metric, only the exact match (node 1)
    // should pass the vector filter. It also has a CITES edge.
    for r in &results {
        assert!(
            r.score > 0.999,
            "All results should exceed 0.999 threshold, got score {} for id {}",
            r.score,
            r.point.id
        );
    }

    // Node 5 (no CITES edge) must not appear even if its score is very high.
    let ids = doc_bindings(&results);
    assert!(
        !ids.contains(&5),
        "Node 5 must not appear (no CITES edge), got: {ids:?}"
    );
}

// =========================================================================
// C. Negative tests (>= 20%)
// =========================================================================

/// GIVEN a graph collection with nodes and edges
/// WHEN a MATCH query uses `similarity()` but the vector param `$v` is missing
///      from the params map
/// THEN the query returns an error (not empty results or a panic).
#[test]
fn test_match_vector_first_missing_vector_param() {
    let (_dir, db) = create_test_db();
    setup_vector_first_collection(&db);

    let sql = "MATCH (doc:Document)-[:CITES]->(ref) \
               WHERE similarity(doc.embedding, $v) > 0.7 \
               RETURN doc LIMIT 10";
    // Pass _collection but NOT the $v vector param.
    let params = collection_param("papers");

    let query =
        velesdb_core::velesql::Parser::parse(sql).expect("test: parse MATCH query (missing param)");

    let err = db
        .execute_query(&query, &params)
        .expect_err("test: missing $v param should produce an error");

    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("param")
            || msg.to_lowercase().contains("vector")
            || msg.to_lowercase().contains("not found")
            || msg.to_lowercase().contains("missing"),
        "Error should indicate missing parameter, got: {msg}"
    );
}

// =========================================================================
// D. Exactness: a seeded walk returns what an unseeded one returns
// =========================================================================
//
// `VectorFirst` seeds the shared MATCH walk from a vector search. It used to
// replace the walk instead: one row per start node holding only the start
// binding, a threshold applied whatever its operator, an `OR` with
// `similarity()` taken as satisfied, the target's labels and properties
// unchecked, and an implicit cap of 100 rows.

/// `g` (graph, cosine, dim 2): document 1 at `[1, 0]` CITES references 10
/// and 11; document 2 at `[0, 1]` CITES reference 12; node 20 is a `Note`
/// cited by document 1.
fn setup_exactness_graph(db: &Database) {
    execute_sql(
        db,
        "CREATE GRAPH COLLECTION g (dimension = 2, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE g");
    let gc = db.get_graph_collection("g").expect("test: g");
    for (id, vector, payload) in [
        (
            1,
            [1.0, 0.0],
            json!({"_labels": ["Document"], "title": "near"}),
        ),
        (
            2,
            [0.0, 1.0],
            json!({"_labels": ["Document"], "title": "far"}),
        ),
        (
            10,
            [1.0, 0.0],
            json!({"_labels": ["Reference"], "year": 2020}),
        ),
        (
            11,
            [1.0, 0.0],
            json!({"_labels": ["Reference"], "year": 2021}),
        ),
        (
            12,
            [1.0, 0.0],
            json!({"_labels": ["Reference"], "year": 2022}),
        ),
        (20, [1.0, 0.0], json!({"_labels": ["Note"]})),
    ] {
        gc.upsert_node(id, &payload, Some(vector.to_vec()))
            .expect("test: node");
    }
    for (id, source, target) in [(100, 1, 10), (101, 1, 11), (102, 2, 12), (103, 1, 20)] {
        gc.add_edge(GraphEdge::new(id, source, target, "CITES").expect("test: edge"))
            .expect("test: add edge");
    }
}

fn run_on_g(db: &Database, sql: &str) -> Vec<SearchResult> {
    let params = HashMap::from([
        ("_collection".to_string(), json!("g")),
        ("q".to_string(), json!([1.0, 0.0])),
    ]);
    execute_sql_with_params(db, sql, &params).expect("test: MATCH")
}

/// The `(alias → id)` binding pairs of each row, as a set of rows.
fn binding_rows(results: &[SearchResult], aliases: &[&str]) -> HashSet<Vec<u64>> {
    results
        .iter()
        .map(|r| {
            let bindings = &r.point.payload.as_ref().expect("test: payload")["_bindings"];
            aliases
                .iter()
                .map(|a| bindings[*a].as_u64().expect("test: binding"))
                .collect()
        })
        .collect()
}

/// GIVEN a near document citing two references
/// WHEN the start node's similarity filters a two-node pattern
/// THEN each matching path is its own row, with both bindings.
#[test]
fn each_path_from_a_similar_start_node_is_a_row() {
    let (_dir, db) = create_test_db();
    setup_exactness_graph(&db);
    let got = run_on_g(
        &db,
        "MATCH (d:Document)-[:CITES]->(r:Reference) WHERE similarity(d.embedding, $q) > 0.7 \
         RETURN d, r LIMIT 10",
    );
    assert_eq!(
        binding_rows(&got, &["d", "r"]),
        HashSet::from([vec![1, 10], vec![1, 11]])
    );
}

/// GIVEN a near document citing a `Note`
/// WHEN the pattern asks for a target label the note lacks
/// THEN the note's path is no row.
#[test]
fn the_target_still_checks_its_labels() {
    let (_dir, db) = create_test_db();
    setup_exactness_graph(&db);
    let got = run_on_g(
        &db,
        "MATCH (d:Document)-[:CITES]->(r:Reference {year: 2021}) \
         WHERE similarity(d.embedding, $q) > 0.7 RETURN d, r LIMIT 10",
    );
    assert_eq!(
        binding_rows(&got, &["d", "r"]),
        HashSet::from([vec![1, 11]])
    );
}

/// GIVEN a near and a far document
/// WHEN WHERE asks for the documents far from the query
/// THEN only the far one matches: the operator is honoured.
#[test]
fn a_less_than_similarity_keeps_the_far_documents() {
    let (_dir, db) = create_test_db();
    setup_exactness_graph(&db);
    let got = run_on_g(
        &db,
        "MATCH (d:Document) WHERE similarity(d.embedding, $q) < 0.5 RETURN d LIMIT 10",
    );
    assert_eq!(binding_rows(&got, &["d"]), HashSet::from([vec![2]]));
}

/// GIVEN a near and a far document
/// WHEN a similarity sits in an OR with a payload predicate
/// THEN the OR is evaluated, not assumed: the far document matches through
///      its title, and nothing else is admitted.
#[test]
fn a_similarity_in_an_or_is_evaluated() {
    let (_dir, db) = create_test_db();
    setup_exactness_graph(&db);
    let got = run_on_g(
        &db,
        "MATCH (d:Document) WHERE similarity(d.embedding, $q) > 0.9 OR d.title = 'far' \
         RETURN d LIMIT 10",
    );
    assert_eq!(
        binding_rows(&got, &["d"]),
        HashSet::from([vec![1], vec![2]])
    );
    let negated = run_on_g(
        &db,
        "MATCH (d:Document) WHERE NOT (similarity(d.embedding, $q) > 0.9) RETURN d LIMIT 10",
    );
    assert_eq!(binding_rows(&negated, &["d"]), HashSet::from([vec![2]]));
}

/// GIVEN 150 documents all similar to the query
/// WHEN a similarity MATCH has no LIMIT
/// THEN every one of them is a row: no implicit cap applies.
#[test]
fn a_similarity_match_without_limit_returns_every_row() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION g (dimension = 2, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE g");
    let gc = db.get_graph_collection("g").expect("test: g");
    for id in 1..=150_u64 {
        gc.upsert_node(id, &json!({"_labels": ["Document"]}), Some(vec![1.0, 0.0]))
            .expect("test: node");
    }
    let got = run_on_g(
        &db,
        "MATCH (d:Document) WHERE similarity(d.embedding, $q) > 0.5 RETURN d",
    );
    assert_eq!(got.len(), 150);
}

/// GIVEN 30 documents nearest the query that cite nothing, and one farther
///      document that does
/// WHEN a similarity MATCH asks for one citing document
/// THEN the seeds grow past the first nearest ones until the walk finds it.
#[test]
fn the_seeds_grow_until_the_limit_is_reached() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION g (dimension = 2, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE g");
    let gc = db.get_graph_collection("g").expect("test: g");
    for id in 1..=30_u64 {
        gc.upsert_node(id, &json!({"_labels": ["Document"]}), Some(vec![1.0, 0.0]))
            .expect("test: near node");
    }
    gc.upsert_node(40, &json!({"_labels": ["Document"]}), Some(vec![0.8, 0.6]))
        .expect("test: farther node");
    gc.upsert_node(41, &json!({"_labels": ["Reference"]}), Some(vec![0.0, 1.0]))
        .expect("test: cited node");
    gc.add_edge(GraphEdge::new(500, 40, 41, "CITES").expect("test: edge"))
        .expect("test: add edge");
    let got = run_on_g(
        &db,
        "MATCH (d:Document)-[:CITES]->(r) WHERE similarity(d.embedding, $q) > 0.5 \
         RETURN d LIMIT 1",
    );
    assert_eq!(
        binding_rows(&got, &["d", "r"]),
        HashSet::from([vec![40, 41]])
    );
}
