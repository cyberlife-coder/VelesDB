//! BDD scenarios: `similarity()` in a browser MATCH, with core's semantics.
//!
//! The browser MATCH path refused every `similarity()` form: it had no
//! vectors to score, because nodes lived only in the graph store. Now that a
//! collection's points are its nodes, their vectors are at hand:
//!
//! - `WHERE similarity(alias.field, $v) <op> t` scores the node the alias
//!   binds (a bare field: the row's last node) and compares like core's MATCH
//!   (`similarity > t` means `distance < t` under a distance metric); the
//!   row's score is the first similarity leaf's.
//! - `executeMatch` with a query vector scores each row's last node, drops
//!   rows below the threshold and orders by score, as REST `/match` does.
//! - `ORDER BY similarity()` is most-similar-first under every metric, and
//!   `RETURN similarity()` projects the score.

use serde_json::json;

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;
use crate::velesql_match_core::{execute_match_rows, MatchScoring};

const QUERY: &str = r#"{"_collection": "kg", "v": [1.0, 0.0]}"#;

/// `kg` (`metric`): author 100 (`[0, 1]`) WROTE docs 1 (`[0.6, 0.8]`),
/// 2 (`[0.8, 0.6]`) and 3 (`[1, 0]`); cosine to `[1, 0]`: 0.6, 0.8, 1.0.
/// The edges are added in ascending doc id, the reverse of the similarity
/// order, so traversal order cannot pass for a sort.
fn kg(metric: &str) -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("kg", 2, metric)
        .expect("test: create kg");
    let store = db.get_shared_store("kg").expect("test: kg store");
    let points: [(u64, [f32; 2], &str); 4] = [
        (100, [0.0, 1.0], "Person"),
        (1, [0.6, 0.8], "Doc"),
        (2, [0.8, 0.6], "Doc"),
        (3, [1.0, 0.0], "Doc"),
    ];
    for (id, vector, label) in points {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &vector,
            Some(json!({"_labels": [label], "name": format!("n{id}")})),
        );
    }
    for doc in 1..=3u64 {
        execute(
            &mut db,
            &format!(
                "INSERT EDGE INTO kg (id = {doc}, source = 100, target = {doc}, label = 'WROTE')"
            ),
            None,
        )
        .expect("test: edge");
    }
    db
}

fn run(db: &mut DatabaseInner, sql: &str) -> Vec<serde_json::Value> {
    execute_match_rows(db, sql, Some(QUERY), None).expect("test: MATCH")
}

fn docs(rows: &[serde_json::Value]) -> Vec<u64> {
    rows.iter()
        .map(|r| {
            r["bindings"]["d"]
                .as_str()
                .and_then(|d| d.parse().ok())
                .expect("test: d is a decimal id string")
        })
        .collect()
}

fn approx(value: &serde_json::Value, expected: f64) -> bool {
    value.as_f64().is_some_and(|v| (v - expected).abs() < 1e-4)
}

#[test]
fn where_similarity_on_an_alias_filters_scores_and_orders() {
    let mut db = kg("cosine");

    let desc = run(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) WHERE similarity(d.embedding, $v) > 0.7 \
         RETURN d.name, similarity() ORDER BY similarity() DESC",
    );
    assert_eq!(docs(&desc), [3, 2], "doc 1 (0.6) is below 0.7");
    assert!(approx(&desc[0]["score"], 1.0) && approx(&desc[1]["score"], 0.8));
    assert!(approx(&desc[0]["projected"]["similarity()"], 1.0));

    let asc = run(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) WHERE similarity(d.embedding, $v) > 0.7 \
         RETURN d.name ORDER BY similarity() ASC",
    );
    assert_eq!(docs(&asc), [2, 3]);
}

#[test]
fn a_bare_similarity_field_scores_the_last_node() {
    let mut db = kg("cosine");

    let rows = run(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) WHERE similarity(embedding, $v) > 0.9 RETURN d.name",
    );
    assert_eq!(docs(&rows), [3]);
}

#[test]
fn similarity_combines_with_a_typed_filter() {
    let mut db = kg("cosine");

    let rows = run(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) WHERE similarity(d.embedding, $v) > 0.7 AND d.name = 'n2' \
         RETURN d.name",
    );
    assert_eq!(docs(&rows), [2]);
}

#[test]
fn under_a_distance_metric_greater_similarity_means_smaller_distance() {
    let mut db = kg("euclidean");

    // Distances to [1, 0]: doc 1 0.894, doc 2 0.632, doc 3 0.0.
    let rows = run(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) WHERE similarity(d.embedding, $v) > 0.7 \
         RETURN d.name ORDER BY similarity() DESC",
    );
    assert_eq!(
        docs(&rows),
        [3, 2],
        "closest first; doc 1 is farther than 0.7"
    );
}

#[test]
fn a_query_vector_scores_the_last_node_and_applies_the_threshold() {
    let mut db = kg("cosine");

    let rows = execute_match_rows(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) RETURN d.name",
        Some(r#"{"_collection": "kg"}"#),
        Some(MatchScoring {
            vector: vec![1.0, 0.0],
            threshold: Some(0.7),
        }),
    )
    .expect("MATCH");
    assert_eq!(
        docs(&rows),
        [3, 2],
        "ordered by score, doc 1 below the threshold"
    );
    assert!(approx(&rows[0]["score"], 1.0));
}

#[test]
fn a_query_vector_of_the_wrong_dimension_is_refused() {
    let mut db = kg("cosine");

    let err = execute_match_rows(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) RETURN d.name",
        Some(r#"{"_collection": "kg"}"#),
        Some(MatchScoring {
            vector: vec![1.0, 0.0, 0.0],
            threshold: None,
        }),
    )
    .expect_err("dimension 3 against 2");
    assert!(err.contains("dimension"), "{err}");
}

#[test]
fn a_threshold_outside_zero_one_is_refused_as_on_rest() {
    let mut db = kg("cosine");

    let err = execute_match_rows(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) RETURN d.name",
        Some(r#"{"_collection": "kg"}"#),
        Some(MatchScoring {
            vector: vec![1.0, 0.0],
            threshold: Some(1.5),
        }),
    )
    .expect_err("threshold 1.5");
    assert!(err.contains("threshold"), "{err}");
}

#[test]
fn a_node_without_a_vector_never_satisfies_similarity() {
    let mut db = kg("cosine");
    execute(
        &mut db,
        "INSERT NODE INTO kg (id = 7, payload = '{\"_labels\": [\"Doc\"]}')",
        None,
    )
    .expect("test: graph-only node");
    execute(
        &mut db,
        "INSERT EDGE INTO kg (id = 7, source = 100, target = 7, label = 'WROTE')",
        None,
    )
    .expect("test: edge to it");

    let rows = run(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) WHERE similarity(d.embedding, $v) >= 0 RETURN d.name",
    );
    assert!(!docs(&rows).contains(&7));
}

/// GIVEN docs a WHERE `similarity()` scores (cosine to `[1, 0]`: 1.0, 0.8)
/// WHEN the MATCH runs through `executeQuery`
/// THEN each returned row's `score` is that similarity, as the ordering
///      used — not the 0 a row built before scoring carried.
#[test]
fn execute_query_rows_carry_the_where_similarity_score() {
    let mut db = kg("cosine");
    let result = execute(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) WHERE similarity(d.embedding, $v) > 0.7 \
         RETURN d ORDER BY similarity() DESC",
        Some(QUERY),
    )
    .expect("test: MATCH");
    let scores: Vec<f32> = result.rows_ref().iter().map(|r| r.score()).collect();
    assert_eq!(scores.len(), 2, "{scores:?}");
    assert!((scores[0] - 1.0).abs() < 1e-5, "{scores:?}");
    assert!((scores[1] - 0.8).abs() < 1e-5, "{scores:?}");
}

/// A row the query vector rejects never stands for its binding: with an
/// unnamed target, author 100's first path (doc 1, 0.6) fails the 0.7
/// threshold and must not hide its later passing paths (docs 2 and 3).
#[test]
fn a_rejected_path_does_not_hide_a_passing_one_of_the_same_binding() {
    let mut db = kg("cosine");

    let rows = execute_match_rows(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(:Doc) RETURN a.name",
        Some(r#"{"_collection": "kg"}"#),
        Some(MatchScoring {
            vector: vec![1.0, 0.0],
            threshold: Some(0.7),
        }),
    )
    .expect("MATCH");
    assert_eq!(rows.len(), 1, "author 100, once: {rows:?}");
}

/// `ORDER BY similarity(d.vector, $v)` scores the vector of the node `d`
/// binds and an arithmetic key reads the target, as core's MATCH ORDER BY
/// does (WASM refused both).
#[test]
fn match_order_by_a_node_vector_and_arithmetic_as_in_core() {
    let mut db = kg("cosine");
    let by_similarity = run(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) RETURN d.name ORDER BY similarity(d.vector, $v) DESC",
    );
    assert_eq!(
        docs(&by_similarity),
        [3, 2, 1],
        "cosine to [1, 0]: 1.0, 0.8, 0.6"
    );
    let by_arithmetic = run(
        &mut db,
        "MATCH (a:Person)-[:WROTE]->(d:Doc) RETURN d.name ORDER BY id * -1 ASC",
    );
    assert_eq!(docs(&by_arithmetic), [3, 2, 1], "-3 < -2 < -1");
}
