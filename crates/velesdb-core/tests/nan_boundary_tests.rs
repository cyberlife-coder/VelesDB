#![cfg(feature = "persistence")]
//! A non-finite vector component cannot enter or query the database (#2106).
//!
//! The dense ingest path validated dimension only. Its sparse sibling has
//! always refused non-finite values, so this is the dense path catching up to a
//! decision the codebase had already made.
//!
//! Why it matters more than "garbage in, garbage out": a NaN in a *query* spoils
//! one call, but a NaN in a *stored* vector is persistent corruption. NaN
//! compares `false` against everything, so one stored NaN makes HNSW's ordering
//! arbitrary for every subsequent query — including queries that are themselves
//! perfectly well-formed. `simd_native::nan_contract_tests` measures the kernel
//! behaviour that follows from letting one through.

use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, Point};

fn seeded(dimension: usize) -> (TempDir, velesdb_core::VectorCollection) {
    let dir = TempDir::new().expect("temp dir");
    let db = Database::open(dir.path()).expect("database");
    db.create_collection("points", dimension, DistanceMetric::Cosine)
        .expect("create collection");
    let collection = db
        .get_vector_collection("points")
        .expect("collection just created");
    (dir, collection)
}

/// Every non-finite value is refused on ingest, and the error names the culprit.
#[test]
fn upsert_refuses_a_non_finite_component() {
    let (_dir, collection) = seeded(4);

    for (label, bad) in [
        ("NaN", f32::NAN),
        ("+inf", f32::INFINITY),
        ("-inf", f32::NEG_INFINITY),
    ] {
        let point = Point::without_payload(1, vec![0.1, bad, 0.3, 0.4]);
        let error = collection
            .upsert(vec![point])
            .expect_err(&format!("{label} must be refused"));
        let message = error.to_string();
        assert!(
            message.contains("index 1"),
            "{label}: the error must name which component is bad, got: {message}"
        );
    }
}

/// A well-formed vector still goes in — the guard rejects, it does not block.
#[test]
fn upsert_still_accepts_a_finite_vector() {
    let (_dir, collection) = seeded(4);
    collection
        .upsert(vec![Point::without_payload(1, vec![0.1, 0.2, 0.3, 0.4])])
        .expect("a finite vector is accepted");
    assert_eq!(collection.len(), 1);
}

/// The refusal covers a batch, not just its first element.
///
/// `upsert` validates the whole batch before storing any of it, so one bad
/// point must reject the batch rather than leave a partial write behind.
#[test]
fn a_bad_point_rejects_the_whole_batch_without_a_partial_write() {
    let (_dir, collection) = seeded(4);

    let batch = vec![
        Point::without_payload(1, vec![0.1, 0.2, 0.3, 0.4]),
        Point::without_payload(2, vec![0.1, 0.2, f32::NAN, 0.4]),
        Point::without_payload(3, vec![0.5, 0.6, 0.7, 0.8]),
    ];
    collection.upsert(batch).expect_err("the batch is refused");

    assert_eq!(
        collection.len(),
        0,
        "no point may survive a refused batch — a partial write is the \
         corruption this guard exists to prevent"
    );
}

/// The read side is closed too: a non-finite query is refused.
///
/// Every typed wrapper delegates to the same `Collection::search`, so one guard
/// there covers `VectorCollection`, `MetadataCollection` and `GraphCollection`
/// alike.
#[test]
fn search_refuses_a_non_finite_query() {
    let (_dir, collection) = seeded(4);
    collection
        .upsert(vec![Point::without_payload(1, vec![0.1, 0.2, 0.3, 0.4])])
        .expect("seed");

    let error = collection
        .search(&[0.1, f32::NAN, 0.3, 0.4], 1)
        .expect_err("a NaN query must be refused");
    assert!(
        error.to_string().contains("index 1"),
        "the error must name the offending component, got: {error}"
    );

    collection
        .search(&[0.1, 0.2, 0.3, 0.4], 1)
        .expect("a finite query still works");
}
