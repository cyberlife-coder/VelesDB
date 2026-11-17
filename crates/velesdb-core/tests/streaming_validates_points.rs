//! A streamed point is checked as an upserted one is, before it is queued.
//!
//! `stream_insert` only queued: a point of the wrong dimension (or with a
//! non-finite component) was accepted, then the drain's `upsert` refused its
//! whole micro-batch — other callers' valid points included — and only logged
//! it. The caller had been told the point was accepted.

#![cfg(feature = "persistence")]

use tempfile::TempDir;
use velesdb_core::collection::streaming::{BackpressureError, StreamingConfig};
use velesdb_core::{Database, DistanceMetric, Point, VectorCollection};

fn streaming_docs(batch_size: usize) -> (TempDir, Database, VectorCollection) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_vector_collection("docs", 4, DistanceMetric::Cosine)
        .expect("test: create");
    let docs = db.get_vector_collection("docs").expect("test: docs");
    docs.enable_streaming(StreamingConfig::new(16, batch_size, 3_600_000))
        .expect("test: enable streaming");
    (dir, db, docs)
}

fn point(id: u64, vector: Vec<f32>) -> Point {
    Point::new(id, vector, None)
}

/// Streaming refuses the points (nothing queued) and the validation a surface
/// runs first names the refusal with core's code.
fn refused_code(
    docs: &VectorCollection,
    points: &[Point],
    result: &Result<impl std::fmt::Debug, BackpressureError>,
) -> &'static str {
    assert!(
        matches!(result, Err(BackpressureError::InvalidPoint)),
        "expected InvalidPoint, got {result:?}"
    );
    docs.validate_points_for_upsert(points)
        .expect_err("the validation refuses what streaming refused")
        .code()
}

#[tokio::test]
async fn a_point_of_the_wrong_dimension_is_refused_before_it_is_queued() {
    let (_dir, _db, docs) = streaming_docs(1);
    let short = point(1, vec![1.0, 0.0, 0.0]);
    let result = docs.stream_insert(short.clone());
    assert_eq!(refused_code(&docs, &[short], &result), "VELES-004");
}

#[tokio::test]
async fn a_non_finite_point_is_refused_before_it_is_queued() {
    let (_dir, _db, docs) = streaming_docs(1);
    let nan = point(1, vec![f32::NAN, 0.0, 0.0, 0.0]);
    let result = docs.stream_insert(nan.clone());
    assert_eq!(refused_code(&docs, &[nan], &result), "VELES-005");
}

/// One bad point refuses the whole batch, and none of it is queued: the
/// valid point beside it does not wait in the buffer to be flushed later.
#[tokio::test]
async fn a_batch_with_one_bad_point_queues_nothing() {
    let (_dir, _db, docs) = streaming_docs(2);
    let good = point(1, vec![1.0, 0.0, 0.0, 0.0]);
    let bad = point(2, vec![1.0, 0.0]);
    let batch = vec![good, bad];
    let result = docs.stream_insert_batch(batch.clone());
    assert_eq!(refused_code(&docs, &batch, &result), "VELES-004");

    // batch_size 2: had point 1 been queued, this second point would flush both.
    docs.stream_insert(point(3, vec![0.0, 1.0, 0.0, 0.0]))
        .expect("test: a valid point is still accepted");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        docs.len(),
        0,
        "point 1 was never queued, so nothing flushed"
    );
}
