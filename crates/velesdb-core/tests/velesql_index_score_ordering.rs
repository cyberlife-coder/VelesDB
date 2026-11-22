//! Original-vector keys and the native search score are different orderings.
#![cfg(feature = "persistence")]

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point, StorageMode};

fn collection(
    metric: DistanceMetric,
    mode: StorageMode,
    points: Vec<Point>,
) -> (TempDir, Database) {
    let dir = tempfile::tempdir().expect("temporary database");
    let db = Database::open(dir.path()).expect("open");
    db.create_vector_collection_with_options("rows", points[0].vector.len(), metric, mode)
        .expect("create");
    db.get_vector_collection("rows")
        .expect("collection")
        .upsert(points)
        .expect("upsert");
    (dir, db)
}

fn run(db: &Database, q: &[f32], tail: &str) -> Vec<(u64, u32)> {
    let sql = format!("SELECT * FROM rows WHERE {tail}");
    let query = Parser::parse(&sql).expect("parse");
    let params = HashMap::from([("q".to_owned(), json!(q))]);
    db.execute_query(&query, &params)
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .into_iter()
        .map(|r| (r.point.id, r.score.to_bits()))
        .collect()
}

fn cosine_points() -> Vec<Point> {
    let vectors = [
        [0.09995, 0.20015, 0.29989],
        [0.09997, 0.20005, 0.29986],
        [0.09987, 0.20011, 0.30002],
    ];
    (1..)
        .zip(vectors)
        .map(|(id, v)| Point::new(id, v.to_vec(), None))
        .collect()
}

#[test]
fn a_secondary_key_keeps_the_full_cosine_scores_on_short_pages() {
    let q = [0.6, 0.1, 1.1, 0.6];
    let points = (1..=128)
        .map(|id| {
            let v = match id {
                1 => [1.1, 0.1, 1.1, 0.6],
                2 => [0.6, 0.1, 1.1, 1.1],
                _ => [0.0, 0.0, 0.0, 1.0],
            };
            Point::new(id, v.to_vec(), Some(json!({ "price": id })))
        })
        .collect();
    let (_dir, db) = collection(DistanceMetric::Cosine, StorageMode::Full, points);
    let ordered = "vector NEAR $q ORDER BY similarity() DESC, price ASC";
    let full = run(&db, &q, &format!("{ordered} LIMIT 128"));
    assert_eq!(full.len(), 128);
    assert_eq!(run(&db, &q, &format!("{ordered} LIMIT 2")), full[..2]);
    for offset in 0..2 {
        let sql = format!("{ordered} LIMIT 1 OFFSET {offset}");
        assert_eq!(run(&db, &q, &sql), full[offset..=offset]);
    }
}

#[test]
fn original_cosines_rank_before_the_near_window_and_the_threshold_window() {
    let q = [0.1, 0.2, 0.3];
    let points = cosine_points();
    let mut original: Vec<_> = points
        .iter()
        .map(|p| {
            (
                p.id,
                DistanceMetric::Cosine.calculate(&p.vector, &q).to_bits(),
            )
        })
        .collect();
    original.sort_by(|(a, x), (b, y)| y.cmp(x).then(a.cmp(b)));
    let (_dir, db) = collection(DistanceMetric::Cosine, StorageMode::Full, points);
    for pred in ["vector NEAR $q", "similarity(vector, $q) > 0.5"] {
        let key = "similarity(vector, $q) DESC";
        assert_eq!(
            run(&db, &q, &format!("{pred} ORDER BY {key} LIMIT 3")),
            original
        );
        assert_eq!(
            run(&db, &q, &format!("{pred} ORDER BY {key} LIMIT 1")),
            original[..1]
        );
    }
    assert_eq!(
        run(
            &db,
            &q,
            "similarity(vector, $q) > 0.5 ORDER BY similarity() DESC LIMIT 1"
        ),
        original[..1]
    );
    let native = run(&db, &q, "vector NEAR $q ORDER BY similarity() DESC LIMIT 3");
    assert_eq!(
        run(
            &db,
            &q,
            "vector NEAR $q ORDER BY similarity() DESC, id ASC LIMIT 3"
        ),
        native
    );
    let mut by_id = native;
    by_id.sort_unstable();
    assert_eq!(
        run(&db, &q, "vector NEAR $q ORDER BY id ASC LIMIT 3"),
        by_id
    );
}

#[test]
fn cosine_search_modes_keep_their_native_score_through_secondary_keys() {
    let q = [0.1, 0.2, 0.3];
    for (mode, codec) in [
        (StorageMode::Full, None),
        (StorageMode::SQ8, Some("sq8")),
        (StorageMode::RaBitQ, Some("rabitq")),
    ] {
        let (_dir, db) = collection(DistanceMetric::Cosine, mode, cosine_points());
        if let Some(codec) = codec {
            let sql = format!("TRAIN QUANTIZER ON rows WITH (m=1, k=2, type={codec})");
            let trained = db
                .execute_query(&Parser::parse(&sql).expect("train"), &HashMap::new())
                .expect("train codec");
            assert_eq!(
                trained[0].point.payload.as_ref().expect("training result")["type"],
                codec
            );
        }
        check_cosine_modes(&db, &q);
    }
}

fn check_cosine_modes(db: &Database, q: &[f32]) {
    for hint in [
        "mode='balanced'",
        "mode='perfect'",
        "mode='accurate'",
        "ef_search=64",
        "rerank=true",
        "rerank=false",
    ] {
        let sql = |order: &str, n: usize| {
            format!("vector NEAR $q ORDER BY {order} LIMIT {n} WITH ({hint})")
        };
        let native = run(db, q, &sql("similarity() DESC", 3));
        assert_eq!(native.len(), 3);
        let mut reversed_ties = native.clone();
        reversed_ties.sort_by(|(a, x), (b, y)| y.cmp(x).then(b.cmp(a)));
        assert_eq!(
            run(db, q, &sql("similarity() DESC, id DESC", 2)),
            reversed_ties[..2],
            "{hint}"
        );
        let mut by_id = native;
        by_id.sort_unstable();
        assert_eq!(run(db, q, &sql("id ASC", 3)), by_id, "{hint}");
    }
}

#[test]
fn a_jaccard_graph_tie_and_the_original_threshold_have_independent_scores() {
    let q = [1.0, 0.0];
    let points = (1..=128)
        .map(|id| {
            let v = match id {
                1 => 1e-9,
                2 => 1e-8,
                128 => 4e-9,
                _ => 0.0,
            };
            Point::new(id, vec![v, 0.0], None)
        })
        .collect();
    let (_dir, db) = collection(DistanceMetric::Jaccard, StorageMode::Full, points);
    let near = "vector NEAR $q ORDER BY";
    let native = run(&db, &q, &format!("{near} similarity() DESC LIMIT 3"));
    assert_eq!(native, [(1, 0), (2, 0), (3, 0)]);
    let original = run(
        &db,
        &q,
        &format!("{near} similarity(vector, $q) DESC LIMIT 3"),
    );
    assert_eq!(
        original,
        [
            (2, 1e-8_f32.to_bits()),
            (128, 4e-9_f32.to_bits()),
            (1, 1e-9_f32.to_bits())
        ]
    );
    for key in ["similarity() DESC, id ASC", "id ASC"] {
        assert_eq!(
            run(&db, &q, &format!("{near} {key} LIMIT 3")),
            native,
            "{key}"
        );
    }
    for key in ["similarity() DESC", "similarity(vector, $q) DESC"] {
        let threshold = format!("similarity(vector, $q) > 0.000000005 ORDER BY {key} LIMIT 1");
        assert_eq!(run(&db, &q, &threshold), original[..1]);
        let threshold = format!("similarity(vector, $q) > 0.000000003 ORDER BY {key} LIMIT 2");
        assert_eq!(run(&db, &q, &threshold), original[..2]);
    }
}
