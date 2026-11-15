//! BDD scenarios for a MATCH whose `WHERE similarity(alias.field, $v) > t`
//! targets a node that is NOT the start of the pattern.
//!
//! The planner runs such a query GraphFirst: it traverses the pattern and
//! checks the similarity as one more WHERE leaf. That check used to compute
//! the score and throw it away, so every row came back with the depth
//! fallback (`1 / (depth + 1)`, i.e. `0.5` at one hop), `RETURN similarity()`
//! projected nothing, and `ORDER BY similarity()` compared equal scores — the
//! ASC and DESC orders were the same traversal order. VectorFirst, which the
//! planner picks when the similarity is on the start node, always reported
//! the real score: the score a row carried depended on the plan.
//!
//! Dataset (`kg`, 2-dim cosine): one author `(100:Person)` who WROTE three
//! docs. The edges are inserted in ascending doc id, the opposite of the
//! similarity order, so traversal order cannot pass for a similarity sort.
//!
//! | doc | vector       | cosine to `[1, 0]` |
//! |-----|--------------|--------------------|
//! | 1   | `[0.6, 0.8]` | 0.6                |
//! | 2   | `[0.8, 0.6]` | 0.8                |
//! | 3   | `[1.0, 0.0]` | 1.0                |

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point, SearchResult};

use super::helpers::{approx_eq, create_test_db, execute_sql_with_params, vector_param};

const QUERY: [f32; 2] = [1.0, 0.0];

fn setup_author_with_docs(db: &Database) {
    db.create_vector_collection("kg", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create kg");
    let vc = db.get_vector_collection("kg").expect("test: get kg");
    vc.upsert(vec![
        Point::new(
            100,
            vec![0.0, 1.0],
            Some(json!({"_labels": ["Person"], "name": "Ada"})),
        ),
        Point::new(
            1,
            vec![0.6, 0.8],
            Some(json!({"_labels": ["Doc"], "name": "d1"})),
        ),
        Point::new(
            2,
            vec![0.8, 0.6],
            Some(json!({"_labels": ["Doc"], "name": "d2"})),
        ),
        Point::new(
            3,
            vec![1.0, 0.0],
            Some(json!({"_labels": ["Doc"], "name": "d3"})),
        ),
    ])
    .expect("test: upsert kg");
    for (edge_id, doc) in [(10u64, 1u64), (11, 2), (12, 3)] {
        vc.add_edge(GraphEdge::new(edge_id, 100, doc, "WROTE").expect("test: edge"))
            .expect("test: add edge");
    }
}

fn run(db: &Database, order: &str) -> Vec<SearchResult> {
    run_above(db, 0.5, order)
}

fn run_above(db: &Database, threshold: f32, order: &str) -> Vec<SearchResult> {
    let sql = format!(
        "MATCH (a:Person)-[:WROTE]->(d:Doc) \
         WHERE similarity(d.embedding, $v) > {threshold} \
         RETURN d.name, similarity() ORDER BY similarity() {order}"
    );
    let mut params = vector_param(&QUERY);
    params.insert("_collection".to_string(), json!("kg"));
    execute_sql_with_params(db, &sql, &params).expect("test: aliased-similarity MATCH must execute")
}

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

/// GIVEN an author whose docs were linked in ascending id (least similar first)
/// WHEN the MATCH orders by `similarity()` DESC, then ASC
/// THEN the rows come back most-similar first, then least-similar first.
#[test]
fn scenario_order_by_similarity_sorts_on_the_aliased_node_score() {
    let (_dir, db) = create_test_db();
    setup_author_with_docs(&db);

    assert_eq!(
        ids(&run(&db, "DESC")),
        vec![3, 2, 1],
        "most similar doc first"
    );
    assert_eq!(
        ids(&run(&db, "ASC")),
        vec![1, 2, 3],
        "least similar doc first"
    );
}

/// GIVEN the same author and docs
/// WHEN the MATCH runs GraphFirst with the similarity on `d`
/// THEN each row's score is the cosine of `d` to the query — not the depth
/// fallback — and `RETURN similarity()` projects that same value.
#[test]
fn scenario_row_score_is_the_similarity_of_the_aliased_node() {
    let (_dir, db) = create_test_db();
    setup_author_with_docs(&db);

    for row in run(&db, "DESC") {
        let expected = match row.point.id {
            1 => 0.6,
            2 => 0.8,
            3 => 1.0,
            other => panic!("unexpected row {other}"),
        };
        assert!(
            approx_eq(row.score, expected, 1e-4),
            "doc {} score {} must be its similarity {expected}",
            row.point.id,
            row.score
        );
        let projected = row
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("similarity()"))
            .and_then(serde_json::Value::as_f64)
            .expect("RETURN similarity() must project the score");
        #[allow(clippy::cast_possible_truncation)]
        let projected = projected as f32;
        assert!(
            approx_eq(projected, expected, 1e-4),
            "doc {} projected similarity() {projected} must be {expected}",
            row.point.id
        );
    }
}

/// GIVEN the same graph in a Euclidean collection, where the score is a
///       distance (lower = more similar)
/// WHEN the MATCH orders by `similarity()` DESC, then ASC
/// THEN DESC is still most-similar first (closest doc first), as for
///      `ORDER BY similarity(field, $v)` and on the SELECT side.
#[test]
fn scenario_order_by_similarity_desc_is_most_similar_first_under_a_distance_metric() {
    let (_dir, db) = create_test_db();
    db.create_vector_collection("kg", 2, velesdb_core::DistanceMetric::Euclidean)
        .expect("test: create kg");
    let vc = db.get_vector_collection("kg").expect("test: get kg");
    vc.upsert(vec![
        Point::new(100, vec![0.0, 1.0], Some(json!({"_labels": ["Person"]}))),
        Point::new(1, vec![0.6, 0.8], Some(json!({"_labels": ["Doc"]}))),
        Point::new(2, vec![0.8, 0.6], Some(json!({"_labels": ["Doc"]}))),
        Point::new(3, vec![1.0, 0.0], Some(json!({"_labels": ["Doc"]}))),
    ])
    .expect("test: upsert kg");
    for (edge_id, doc) in [(10u64, 1u64), (11, 2), (12, 3)] {
        vc.add_edge(GraphEdge::new(edge_id, 100, doc, "WROTE").expect("test: edge"))
            .expect("test: add edge");
    }

    // Under a distance metric `similarity(...) > 1.0` keeps distance < 1.0:
    // all three docs (0.894, 0.632, 0.0 from the query).
    assert_eq!(
        ids(&run_above(&db, 1.0, "DESC")),
        vec![3, 2, 1],
        "closest doc first"
    );
    assert_eq!(
        ids(&run_above(&db, 1.0, "ASC")),
        vec![1, 2, 3],
        "farthest doc first"
    );
}
