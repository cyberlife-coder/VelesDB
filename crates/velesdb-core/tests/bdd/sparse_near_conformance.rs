//! BDD conformance for the `SPARSE_NEAR` VelesQL surface.
//!
//! Contract under test: `WHERE vector SPARSE_NEAR {..}` ranks documents by the
//! exact sparse inner product (dot) between the query and each document's
//! default ("") sparse vector, returns them sorted descending, truncated to
//! `LIMIT`, and carries the dot product through as `SearchResult::score`.
//!
//! Ground-truth ranking is hand-computed and verified against the sparse search
//! path (`sparse_index/search`): the corpus stays well under
//! `SMALL_CORPUS_LINEAR_THRESHOLD` (100_000), so every query takes the dense
//! linear-scan path, which sums `query_weight * doc_weight` per overlapping term
//! and applies NO `score > 0` filter — negative scores are kept.

use std::collections::{BTreeMap, HashMap};

use serde_json::json;
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::{Database, GraphEdge, Point, SearchResult};

use super::helpers::{approx_eq, create_test_db, execute_sql, execute_sql_with_params, result_ids};

/// Float epsilon for sparse dot-product score comparison.
const EPS: f32 = 1e-5;

/// Builds a default-index ("") sparse vector from `(term, weight)` pairs.
fn sparse_default(pairs: Vec<(u32, f32)>) -> BTreeMap<String, SparseVector> {
    let mut map = BTreeMap::new();
    map.insert(String::new(), SparseVector::new(pairs));
    map
}

/// Builds a point carrying only a default sparse vector (dense vector is a
/// fixed placeholder; SPARSE_NEAR ignores it).
fn sparse_point(id: u64, pairs: Vec<(u32, f32)>) -> Point {
    Point {
        id,
        vector: vec![1.0, 0.0],
        payload: Some(json!({ "id": id })),
        sparse_vectors: Some(sparse_default(pairs)),
    }
}

/// GIVEN the `hc` corpus (dim 2), default sparse index:
///   id1 [(10,5.0),(20,1.0)] · id2 [(10,2.0),(30,4.0)]
///   id3 [(20,3.0),(30,3.0)] · id4 [(40,9.0)]
fn setup_hc(db: &Database) {
    db.create_vector_collection("hc", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create hc collection");
    let vc = db.get_vector_collection("hc").expect("test: get hc");
    vc.upsert(vec![
        sparse_point(1, vec![(10, 5.0), (20, 1.0)]),
        sparse_point(2, vec![(10, 2.0), (30, 4.0)]),
        sparse_point(3, vec![(20, 3.0), (30, 3.0)]),
        sparse_point(4, vec![(40, 9.0)]),
    ])
    .expect("test: upsert hc");
}

/// Maps results to their `(id, score)` pairs in returned order.
fn id_scores(results: &[SearchResult]) -> Vec<(u64, f32)> {
    results.iter().map(|r| (r.point.id, r.score)).collect()
}

/// WHEN `SPARSE_NEAR {10:1.0,30:2.0} LIMIT 3` THEN docs rank by exact dot:
/// id2=1*2+2*4=10, id3=2*3=6, id1=1*5=5; id4 has no overlap (absent).
/// Ground truth: hand-computed inner products; scores strictly distinct so the
/// (unstable) sort yields a deterministic id order.
#[test]
fn test_sparse_near_exact_dot_ranking() {
    let (_dir, db) = create_test_db();
    setup_hc(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hc WHERE vector SPARSE_NEAR {10: 1.0, 30: 2.0} LIMIT 3",
    )
    .expect("test: sparse_near literal");

    let got = id_scores(&results);
    let ids: Vec<u64> = got.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![2, 3, 1], "dot ranking id2(10) > id3(6) > id1(5)");
    assert!(approx_eq(got[0].1, 10.0, EPS), "id2 dot = 1*2+2*4 = 10");
    assert!(approx_eq(got[1].1, 6.0, EPS), "id3 dot = 2*3 = 6");
    assert!(approx_eq(got[2].1, 5.0, EPS), "id1 dot = 1*5 = 5");
    assert!(
        !result_ids(&results).contains(&4),
        "id4 shares no term with the query: dot = 0, never scored"
    );
}

/// WHEN the same query is capped at `LIMIT 2` THEN only the top two survive the
/// post-rank truncation. Ground truth: top-k truncation after descending sort.
#[test]
fn test_sparse_near_limit_truncates_after_rank() {
    let (_dir, db) = create_test_db();
    setup_hc(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hc WHERE vector SPARSE_NEAR {10: 1.0, 30: 2.0} LIMIT 2",
    )
    .expect("test: sparse_near limit 2");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![2, 3], "LIMIT 2 keeps only the two highest dots");
}

/// WHEN the query vector is bound as a `$sv` parameter (shorthand JSON object
/// `{"10":1.0,"30":2.0}`) THEN ranking is identical to the literal form.
/// Ground truth: `SparseVectorExpr::resolve` parses an object whose keys are u32
/// strings and whose values are weights into the same `SparseVector`.
#[test]
fn test_sparse_near_param_matches_literal() {
    let (_dir, db) = create_test_db();
    setup_hc(&db);

    let mut params = std::collections::HashMap::new();
    params.insert("sv".to_string(), json!({ "10": 1.0, "30": 2.0 }));
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM hc WHERE vector SPARSE_NEAR $sv LIMIT 3",
        &params,
    )
    .expect("test: sparse_near param");

    let got = id_scores(&results);
    let ids: Vec<u64> = got.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        ids,
        vec![2, 3, 1],
        "param ranking equals the literal ranking"
    );
    assert!(approx_eq(got[0].1, 10.0, EPS), "id2 dot = 10");
    assert!(approx_eq(got[1].1, 6.0, EPS), "id3 dot = 6");
    assert!(approx_eq(got[2].1, 5.0, EPS), "id1 dot = 5");
}

/// WHEN the query is bound via the structured `{indices, values}` JSON shape
/// THEN it resolves to the same sparse vector and ranking as the shorthand.
/// Ground truth: `try_parse_structured_sparse` zips equal-length arrays.
#[test]
fn test_sparse_near_param_structured_shape() {
    let (_dir, db) = create_test_db();
    setup_hc(&db);

    let mut params = std::collections::HashMap::new();
    params.insert(
        "sv".to_string(),
        json!({ "indices": [10, 30], "values": [1.0, 2.0] }),
    );
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM hc WHERE vector SPARSE_NEAR $sv LIMIT 3",
        &params,
    )
    .expect("test: sparse_near structured param");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![2, 3, 1],
        "structured param yields the same ranking"
    );
    assert!(
        approx_eq(results[0].score, 10.0, EPS),
        "structured param id2 dot = 10"
    );
}

/// WHEN a `MATCH (d)-[:CITES]->(x)` anchor is AND-required, retrieval is
/// exhaustive *within the graph matches*: only id1 has an outgoing CITES edge,
/// so it is returned despite ranking 3rd globally (dot 5 < id2 10 < id3 6).
/// Ground truth: anchor set = {1}; the sparse per-id filter keeps only id1.
#[test]
fn test_sparse_near_graph_anchor_narrows_to_edge_source() {
    let (_dir, db) = create_test_db();
    setup_hc(&db);
    let vc = db.get_vector_collection("hc").expect("test: get hc");
    vc.add_edge(GraphEdge::new(900, 1, 2, "CITES").expect("test: edge"))
        .expect("test: add edge");

    let results = execute_sql(
        &db,
        "SELECT * FROM hc AS d WHERE vector SPARSE_NEAR {10: 1.0, 30: 2.0} \
         AND MATCH (d)-[:CITES]->(x) LIMIT 1",
    )
    .expect("test: sparse_near + anchor");

    assert_eq!(results.len(), 1, "exactly the single anchor is returned");
    assert_eq!(
        results[0].point.id, 1,
        "id1 is the only CITES source; anchor overrides its 3rd-place global rank"
    );
    assert!(approx_eq(results[0].score, 5.0, EPS), "id1 dot = 1*5 = 5");
}

/// WHEN a document carries a NEGATIVE term weight, the sparse path KEEPS its
/// negative score (no `score > 0` filter exists; `topk_push` admits any doc
/// while the heap is below k). id5 [(10,-2.0),(50,3.0)] under query {10:1.0}
/// scores -2.0 and ranks last but is still returned.
/// Ground truth: id1=5, id2=2, id5=-2; strictly distinct → deterministic order.
/// VERIFIED: `sparse_index/search/mod.rs` has no positive-score filter; the
/// `has_negative_weight` branch only changes the search STRATEGY (linear scan),
/// not result filtering. CORRECT behavior under inner-product semantics is to
/// keep negatives, so this is a faithful conformance assertion, not a bug lock.
#[test]
fn test_sparse_near_keeps_negative_scores() {
    let (_dir, db) = create_test_db();
    setup_hc(&db);
    let vc = db.get_vector_collection("hc").expect("test: get hc");
    vc.upsert(vec![sparse_point(5, vec![(10, -2.0), (50, 3.0)])])
        .expect("test: upsert id5");

    let results = execute_sql(
        &db,
        "SELECT * FROM hc WHERE vector SPARSE_NEAR {10: 1.0} LIMIT 10",
    )
    .expect("test: sparse_near negative");

    let got = id_scores(&results);
    let ids: Vec<u64> = got.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        ids,
        vec![1, 2, 5],
        "only docs touching term 10; id5 kept despite score < 0"
    );
    assert!(approx_eq(got[0].1, 5.0, EPS), "id1 dot = 1*5 = 5");
    assert!(approx_eq(got[1].1, 2.0, EPS), "id2 dot = 1*2 = 2");
    assert!(
        approx_eq(got[2].1, -2.0, EPS),
        "id5 dot = 1*(-2) = -2 (negative kept)"
    );
    assert!(
        !result_ids(&results).contains(&3) && !result_ids(&results).contains(&4),
        "id3/id4 do not contain term 10 → never scored"
    );
}

/// GIVEN three points whose sparse scores rank 3 > 2 > 1, categories A, A, B
/// (dense vectors: 1 is the query's own, 2 close to it, 3 orthogonal).
fn setup_categories(db: &Database) {
    db.create_vector_collection("cats", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create cats collection");
    let point = |id: u64, vector: Vec<f32>, cat: &str, weight: f32| Point {
        id,
        vector,
        payload: Some(json!({ "cat": cat })),
        sparse_vectors: Some(sparse_default(vec![(7, weight)])),
    };
    db.get_vector_collection("cats")
        .expect("test: get cats")
        .upsert(vec![
            point(1, vec![1.0, 0.0], "A", 3.0),
            point(2, vec![0.8, 0.6], "A", 2.0),
            point(3, vec![0.0, 1.0], "B", 1.0),
        ])
        .expect("test: upsert cats");
}

fn categories(results: &[SearchResult]) -> Vec<(String, Option<i64>)> {
    results
        .iter()
        .map(|r| {
            let payload = r.point.payload.as_ref().expect("test: payload");
            (
                payload["cat"].as_str().expect("test: cat").to_string(),
                payload.get("count").and_then(serde_json::Value::as_i64),
            )
        })
        .collect()
}

/// WHEN `SELECT DISTINCT cat … SPARSE_NEAR … LIMIT 2` THEN both categories
/// come back: LIMIT bounds the distinct rows, not the hits DISTINCT reads
/// (the fetch was cut at the two A hits, and one row came back). The same
/// for a hybrid NEAR + SPARSE_NEAR and for a NEAR alone.
#[test]
fn test_distinct_over_sparse_near_reads_past_the_window() {
    let (_dir, db) = create_test_db();
    setup_categories(&db);
    let params = HashMap::from([("q".to_string(), json!([1.0, 0.0]))]);
    for near in ["", "vector NEAR $q AND "] {
        let sql = format!(
            "SELECT DISTINCT cat FROM cats WHERE {near}vector SPARSE_NEAR {{7: 1.0}} LIMIT 2"
        );
        let rows = execute_sql_with_params(&db, &sql, &params).expect("test: distinct");
        let cats: Vec<String> = categories(&rows).into_iter().map(|(c, _)| c).collect();
        assert_eq!(cats, vec!["A".to_string(), "B".to_string()], "{sql}");
    }
    let rows = execute_sql_with_params(
        &db,
        "SELECT DISTINCT cat FROM cats WHERE vector NEAR $q LIMIT 2",
        &params,
    )
    .expect("test: distinct near");
    assert_eq!(rows.len(), 2);
}

/// WHEN a vector `GROUP BY` sits beside `SPARSE_NEAR` THEN the hits are
/// grouped, as beside a `NEAR`: the sparse path returned them ungrouped.
#[test]
fn test_vector_group_by_over_sparse_near_groups_the_hits() {
    let (_dir, db) = create_test_db();
    setup_categories(&db);
    let params = HashMap::from([("q".to_string(), json!([1.0, 0.0]))]);
    for near in ["", "vector NEAR $q AND "] {
        let sql = format!(
            "SELECT cat, COUNT(*) FROM cats WHERE {near}vector SPARSE_NEAR {{7: 1.0}} GROUP BY cat LIMIT 2"
        );
        let rows = execute_sql_with_params(&db, &sql, &params).expect("test: group by");
        let mut groups = categories(&rows);
        groups.sort();
        assert_eq!(
            groups,
            vec![("A".to_string(), Some(2)), ("B".to_string(), Some(1))],
            "{sql}"
        );
    }
}

/// Runs `sql` over the `cats` corpus with `$q = [1, 0]` (similarity 1, 0.8
/// and 0 to points 1, 2 and 3) and returns `(id, score)` in order.
fn cats_rows(sql: &str) -> Vec<(u64, f32)> {
    let (_dir, db) = create_test_db();
    setup_categories(&db);
    let params = HashMap::from([("q".to_string(), json!([1.0, 0.0]))]);
    id_scores(&execute_sql_with_params(&db, sql, &params).unwrap_or_else(|e| panic!("{sql}: {e}")))
}

/// WHEN a `similarity()` threshold sits beside `SPARSE_NEAR`, alone or with a
/// `NEAR` THEN it bounds the points either branch returns: point 3
/// (similarity 0) fails `> 0.5` and is not returned — the threshold used to
/// be dropped.
#[test]
fn test_similarity_threshold_bounds_sparse_near() {
    let sparse = cats_rows(
        "SELECT * FROM cats WHERE vector SPARSE_NEAR {7: 1.0} AND similarity(vector, $q) > 0.5 LIMIT 5",
    );
    assert_eq!(sparse.iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 2]);
    assert!(
        approx_eq(sparse[0].1, 3.0, EPS) && approx_eq(sparse[1].1, 2.0, EPS),
        "{sparse:?}"
    );
    let hybrid = cats_rows(
        "SELECT * FROM cats WHERE vector NEAR $q AND vector SPARSE_NEAR {7: 1.0} \
         AND similarity(vector, $q) > 0.5 LIMIT 5",
    );
    assert_eq!(hybrid.iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 2]);
}

/// WHEN the threshold sits under `OR` or `NOT` beside `SPARSE_NEAR` THEN it
/// composes with the other predicates and the survivors rank by the sparse
/// score — the query used to run the `similarity()` scan, dropping
/// `SPARSE_NEAR` and scoring by similarity.
#[test]
fn test_similarity_threshold_under_or_and_not_keeps_sparse_near() {
    let or = cats_rows(
        "SELECT * FROM cats WHERE vector SPARSE_NEAR {7: 1.0} \
         AND (similarity(vector, $q) > 0.9 OR cat = 'B') LIMIT 5",
    );
    assert_eq!(or.iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 3]);
    assert!(
        approx_eq(or[0].1, 3.0, EPS) && approx_eq(or[1].1, 1.0, EPS),
        "{or:?}"
    );
    let not = cats_rows(
        "SELECT * FROM cats WHERE vector SPARSE_NEAR {7: 1.0} \
         AND NOT similarity(vector, $q) > 0.9 LIMIT 5",
    );
    assert_eq!(not.iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 3]);
    assert!(
        approx_eq(not[0].1, 2.0, EPS) && approx_eq(not[1].1, 1.0, EPS),
        "{not:?}"
    );
}
