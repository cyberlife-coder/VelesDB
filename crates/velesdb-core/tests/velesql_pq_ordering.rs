//! ORDER BY on original vectors and on the search's primary score across a PQ cut.
#![cfg(feature = "persistence")]

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point, SearchResult};

fn point(id: u64, x: f32, y: f32) -> Point {
    Point::new(id, vec![x, y], Some(json!({"kind": "row"})))
}

fn run(db: &Database, sql: &str) -> Vec<SearchResult> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("parse {sql}: {e}"));
    let params = HashMap::from([("q".to_owned(), json!([1.0, 0.0]))]);
    db.execute_query(&query, &params)
        .unwrap_or_else(|e| panic!("execute {sql}: {e}"))
}

fn setup(codec: &str) -> (TempDir, Database) {
    let dir = tempfile::tempdir().expect("temporary database");
    {
        let db = Database::open(dir.path()).expect("create database");
        db.create_vector_collection("rows", 2, DistanceMetric::Euclidean)
            .expect("create collection");
        let c = db.get_vector_collection("rows").expect("rows");
        // Three training points for three centroids make the codebook independent
        // of centroid initialization. Each training point represents itself.
        c.upsert(vec![
            point(1, -3.0, 0.0),
            point(2, 5.1, 0.0),
            point(3, 1.0, 4.2),
        ])
        .expect("training points");
        run(
            &db,
            &format!("TRAIN QUANTIZER ON rows WITH (m=1, k=3, oversampling=16, type={codec})"),
        );
        c.flush().expect("flush training points");
    }
    let db = Database::open(dir.path()).expect("restore trained quantizer");
    let c = db.get_vector_collection("rows").expect("rows");
    c.upsert(vec![point(9, 1.0, 0.3)]).expect("fourth row");
    // The actual search sees all four rows. Their nearest f32 vector (#9)
    // shares a code with #3, behind *two distinct* ADC distances.
    let hits = c.search(&[1.0, 0.0], 4).expect("default search");
    assert_eq!(hits.len(), 4);
    assert_eq!(hits[0].point.id, 1);
    assert_eq!(hits[1].point.id, 2);
    let nearest = hits
        .iter()
        .find(|r| r.point.id == 9)
        .expect("search saw #9");
    assert!((nearest.score - 4.2).abs() < 1e-5, "{codec}");
    (dir, db)
}

fn assert_order(db: &Database, tail: &str, ids: &[u64]) {
    let sql = format!("SELECT * FROM rows WHERE {tail}");
    let got: Vec<_> = run(db, &sql).into_iter().map(|r| r.point.id).collect();
    assert_eq!(got, ids, "{sql}");
}

#[test]
fn explicit_similarity_ranks_original_vectors_before_the_pq_cut() {
    for codec in ["pq", "opq"] {
        let (_dir, db) = setup(codec);
        let order = "vector NEAR $q ORDER BY similarity(vector, $q) DESC";
        assert_order(&db, &format!("{order} LIMIT 1"), &[9]);
        assert_order(&db, &format!("{order} LIMIT 1 OFFSET 1"), &[1]);
        assert_order(&db, &format!("{order} LIMIT 4"), &[9, 1, 2, 3]);
        assert_order(&db, &format!("{order}, id ASC LIMIT 1"), &[9]);
    }
}

#[test]
fn bare_similarity_and_score_expressions_keep_the_admitted_pq_score() {
    for codec in ["pq", "opq"] {
        let (_dir, db) = setup(codec);
        let order = "vector NEAR $q ORDER BY";
        for (keys, ids) in [
            ("similarity() DESC LIMIT 1", &[1][..]),
            ("similarity() DESC, id ASC LIMIT 1", &[1]),
            ("similarity() ASC LIMIT 4", &[3, 9, 2, 1]),
            ("similarity() * 1 DESC, id DESC LIMIT 1", &[9]),
            ("vector_score * 1 DESC, id DESC LIMIT 1", &[9]),
        ] {
            assert_order(&db, &format!("{order} {keys}"), ids);
        }
        let rows = run(
            &db,
            "SELECT * FROM rows WHERE vector NEAR $q ORDER BY id DESC LIMIT 1",
        );
        assert_eq!(rows[0].point.id, 9);
        assert!((rows[0].score - 4.2).abs() < 1e-5, "{codec}");
    }
}

#[test]
fn exact_search_options_and_similarity_thresholds_keep_their_scores() {
    let (_dir, db) = setup("pq");
    for hint in [
        "mode='balanced'",
        "mode='perfect'",
        "rerank=true",
        "ef_search=64",
    ] {
        let tail =
            format!("vector NEAR $q ORDER BY similarity() DESC, id ASC LIMIT 1 WITH ({hint})");
        assert_order(&db, &tail, &[9]);
    }
    assert_order(
        &db,
        "similarity(vector, $q) > 1.0 ORDER BY similarity() DESC, id ASC LIMIT 1",
        &[9],
    );
    assert_order(
        &db,
        "similarity(vector, $q) > 1.0 ORDER BY similarity(vector, $q) DESC LIMIT 1",
        &[9],
    );
}

#[test]
fn a_similarity_threshold_uses_original_vectors_beyond_the_adc_prefix() {
    let (_dir, db) = setup("pq");
    let c = db.get_vector_collection("rows").expect("rows");
    c.upsert((10..50).map(|id| point(id, -3.0, 0.0)).collect::<Vec<_>>())
        .expect("vectors with the nearest code");
    let prefix = c.search(&[1.0, 0.0], 20).expect("default ADC prefix");
    assert_eq!(prefix.len(), 20);
    assert!(prefix.iter().all(|r| r.point.id != 9));
    // The only vector closer than one is #9. Its centroid is farther than four.
    for key in ["similarity()", "similarity(vector, $q)"] {
        assert_order(
            &db,
            &format!("similarity(vector, $q) > 1.0 ORDER BY {key} DESC LIMIT 1"),
            &[9],
        );
    }
    let near = "SELECT * FROM rows WHERE vector NEAR $q AND similarity(vector, $q) > 1.0";
    for (order, expected_score) in [
        ("similarity() DESC", 4.2),
        ("similarity() DESC, id ASC", 4.2),
        ("similarity(vector, $q) DESC", 0.3),
    ] {
        let sql = format!("{near} ORDER BY {order} LIMIT 1");
        let rows = run(&db, &sql);
        assert_eq!(
            rows.iter().map(|r| r.point.id).collect::<Vec<_>>(),
            [9],
            "{sql}"
        );
        assert!((rows[0].score - expected_score).abs() < 1e-5, "{sql}");
    }
}

#[test]
fn an_indexed_filter_and_a_disabled_rescore_keep_their_native_scores() {
    let (_dir, db) = setup("pq");
    let c = db.get_vector_collection("rows").expect("rows");
    let filtered = "vector NEAR $q AND kind = 'row' ORDER BY similarity() DESC";
    for (indexed, winner) in [(false, 1), (true, 9)] {
        if indexed {
            c.create_index("kind").expect("index filter");
        }
        assert_order(&db, &format!("{filtered} LIMIT 1"), &[winner]);
        assert_order(&db, &format!("{filtered}, id ASC LIMIT 1"), &[winner]);
    }
    c.apply_advanced_config(Some(Some(0)), None, None)
        .expect("disable ADC");
    assert_order(
        &db,
        "vector NEAR $q ORDER BY similarity() DESC, id ASC LIMIT 1",
        &[9],
    );
}

#[test]
fn pending_vectors_keep_the_same_exact_score_as_the_native_search() {
    use velesdb_core::collection::streaming::DeferredIndexerConfig;

    let (_dir, db) = setup("pq");
    let c = db.get_vector_collection("rows").expect("rows");
    c.apply_advanced_config(
        None,
        Some(Some(DeferredIndexerConfig {
            enabled: true,
            merge_threshold: 1000,
            max_buffer_age_ms: u64::MAX,
        })),
        None,
    )
    .expect("start deferring");
    c.upsert(vec![point(10, 1.0, 0.2)]).expect("pending vector");
    let hits = c.search(&[1.0, 0.0], 1).expect("native pending search");
    assert_eq!(hits[0].point.id, 10);
    assert!((hits[0].score - 0.2).abs() < 1e-5);
    let rows = run(
        &db,
        "SELECT * FROM rows WHERE vector NEAR $q ORDER BY similarity() DESC, id ASC LIMIT 2",
    );
    assert_eq!(rows.iter().map(|r| r.point.id).collect::<Vec<_>>(), [10, 1]);
    assert!((rows[0].score - 0.2).abs() < 1e-5);
}
