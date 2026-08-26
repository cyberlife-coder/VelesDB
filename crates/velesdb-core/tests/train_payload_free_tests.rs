#![cfg(feature = "persistence")]
//! `TRAIN QUANTIZER` must see a collection's vectors, payload or not.
//!
//! Training enumerates the collection's ids before extracting vectors, and
//! `Collection::all_ids` is the *payload* store's view — its own doc says
//! "Only returns IDs that have payload entries stored. Points inserted with
//! `None` payload may not appear." A collection used for pure vector search
//! carries no payloads, so that view is empty and training reported "no
//! vectors available" about a collection holding its full count.
//!
//! `all_point_ids` unions vector and payload storage and is documented as the
//! authoritative set.

use std::collections::HashMap;

use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point, StorageMode};

const DIM: usize = 8;
const POINTS: u64 = 256;

fn vector_for(i: u64) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..DIM)
        .map(|d| ((i as f32) * 0.37 + (d as f32) * 1.13).sin())
        .collect()
}

/// Seeds a collection whose points carry `None` payload.
fn payload_free_db(name: &str) -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_collection(name, DIM, DistanceMetric::Cosine)
        .expect("test: create collection");

    let coll = db
        .get_vector_collection(name)
        .expect("test: collection must exist");
    let points: Vec<Point> = (0..POINTS)
        .map(|i| Point::without_payload(i, vector_for(i)))
        .collect();
    coll.upsert(points).expect("test: upsert");

    (dir, db)
}

/// The two id views disagree, and only one of them is authoritative.
#[test]
fn payload_free_points_are_invisible_to_the_payload_id_view() {
    let (_dir, db) = payload_free_db("vecs");
    let coll = db.get_vector_collection("vecs").expect("test: exists");

    assert_eq!(
        coll.all_ids().len(),
        0,
        "all_ids reads the payload store, which is empty here"
    );
    assert_eq!(
        u64::try_from(coll.all_point_ids().len()).expect("test: id count fits in u64"),
        POINTS,
        "all_point_ids unions vector storage and sees every point"
    );
}

/// Training must succeed on a collection that carries no payloads.
#[test]
fn train_pq_succeeds_on_a_payload_free_collection() {
    let (_dir, db) = payload_free_db("vecs");

    let query = Parser::parse("TRAIN QUANTIZER ON vecs WITH (m=2, k=4)").expect("test: parse");
    db.execute_query(&query, &HashMap::new())
        .expect("test: a collection full of vectors must be trainable");

    let coll = db.get_vector_collection("vecs").expect("test: exists");
    assert_eq!(
        coll.config().storage_mode,
        StorageMode::ProductQuantization,
        "training must have flipped the storage mode"
    );
}

/// The same holds for the other quantizer types, which share the extraction.
#[test]
fn every_quantizer_type_trains_on_a_payload_free_collection() {
    for (name, sql) in [
        (
            "opq_c",
            "TRAIN QUANTIZER ON opq_c WITH (type='opq', m=2, k=4)",
        ),
        (
            "rabitq_c",
            "TRAIN QUANTIZER ON rabitq_c WITH (type='rabitq')",
        ),
        ("sq8_c", "TRAIN QUANTIZER ON sq8_c WITH (type='sq8')"),
    ] {
        let (_dir, db) = payload_free_db(name);
        let query = Parser::parse(sql).expect("test: parse");
        db.execute_query(&query, &HashMap::new())
            .unwrap_or_else(|e| {
                panic!("test: {sql} must succeed on a payload-free collection: {e}")
            });
    }
}

/// A genuinely empty collection still reports that it has nothing to train on.
#[test]
fn an_empty_collection_still_reports_no_vectors() {
    let dir = TempDir::new().expect("test: temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_collection("empty_c", DIM, DistanceMetric::Cosine)
        .expect("test: create collection");

    let query = Parser::parse("TRAIN QUANTIZER ON empty_c WITH (m=2, k=4)").expect("test: parse");
    assert!(
        db.execute_query(&query, &HashMap::new()).is_err(),
        "an empty collection must still fail training"
    );
}
