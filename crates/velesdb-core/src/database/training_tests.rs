//! Tests for `TRAIN QUANTIZER` parameter handling (`training.rs`).

use super::*;
use crate::point::Point;
use crate::velesql::Parser;
use crate::{DistanceMetric, SearchResult, StorageMode};
use std::collections::HashMap;
use tempfile::tempdir;

fn seeded_db(name: &str) -> (tempfile::TempDir, Database) {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection(name, 16, DistanceMetric::Euclidean)
        .unwrap();
    let points: Vec<Point> = (0..300_u64)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v: Vec<f32> = (0..16_u64)
                .map(|d| ((i * 31 + d * 17 + 11) % 1000) as f32 / 1000.0)
                .collect();
            Point::new(i, v, None)
        })
        .collect();
    db.get_vector_collection(name)
        .unwrap()
        .upsert(points)
        .unwrap();
    (dir, db)
}

fn train(db: &Database, sql: &str) -> Result<Vec<SearchResult>> {
    db.execute_query(&Parser::parse(sql).unwrap(), &HashMap::new())
}

/// `opq=true` is not a TRAIN parameter — the spelling is `type=opq`. It used
/// to be ignored, so the statement trained plain PQ and reported success.
#[test]
fn an_unknown_with_key_is_refused_instead_of_ignored() {
    let (_dir, db) = seeded_db("docs");

    let err = train(&db, "TRAIN QUANTIZER ON docs WITH (m=4, k=16, opq=true)")
        .expect_err("an unknown TRAIN parameter must be refused");

    assert!(matches!(err, Error::InvalidQuantizerConfig(_)), "{err:?}");
    let message = err.to_string();
    assert!(message.contains("'opq'"), "must name the key: {message}");
    assert!(
        message.contains("m, k, type, oversampling, sample, force"),
        "must list the supported keys: {message}"
    );
    assert!(
        db.get_vector_collection("docs")
            .unwrap()
            .config()
            .storage_mode
            != StorageMode::ProductQuantization,
        "a refused statement must not train anything"
    );
}

/// A misspelt parameter fails loudly instead of falling back to its default.
#[test]
fn a_misspelt_with_key_is_refused() {
    let (_dir, db) = seeded_db("docs");

    let err = train(&db, "TRAIN QUANTIZER ON docs WITH (m=4, k=16, froce=true)")
        .expect_err("a misspelt TRAIN parameter must be refused");

    assert!(err.to_string().contains("'froce'"), "{err}");
}

/// Positive control: every documented parameter is still accepted together.
#[test]
fn every_documented_with_key_is_accepted() {
    let (_dir, db) = seeded_db("docs");

    let results = train(
        &db,
        "TRAIN QUANTIZER ON docs WITH (m=4, k=16, type=opq, oversampling=4, sample=200, force=true)",
    )
    .expect("every documented TRAIN parameter must be accepted");

    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["type"], serde_json::json!("opq"));
    assert_eq!(
        db.get_vector_collection("docs")
            .unwrap()
            .config()
            .storage_mode,
        StorageMode::ProductQuantization
    );
}
