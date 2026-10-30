//! `Database::refresh_storage_gauges` feeds `velesdb_documents_total` and
//! `velesdb_index_size_bytes`, which nothing ever set before.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::Ordering;

use tempfile::tempdir;

use crate::metrics::OperationalMetrics;
use crate::point::Point;
use crate::{Database, DistanceMetric};

#[test]
fn storage_gauges_count_every_collection_kind() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .unwrap();
    db.create_metadata_collection("meta").unwrap();
    let docs = db.get_vector_collection("docs").unwrap();
    docs.upsert(vec![
        Point::new(1, vec![1.0, 0.0], Some(serde_json::json!({"tag": "a"}))),
        Point::new(2, vec![0.0, 1.0], Some(serde_json::json!({"tag": "b"}))),
    ])
    .unwrap();
    db.get_metadata_collection("meta")
        .unwrap()
        .upsert(vec![Point::metadata_only(7, serde_json::json!({"k": 1}))])
        .unwrap();

    let metrics = OperationalMetrics::new();
    db.refresh_storage_gauges(&metrics);

    assert_eq!(metrics.documents_total.load(Ordering::Relaxed), 3);
    assert_eq!(
        metrics.index_size_bytes.load(Ordering::Relaxed),
        u64::try_from(docs.indexes_memory_usage()).unwrap()
            + u64::try_from(
                db.get_metadata_collection("meta")
                    .unwrap()
                    .inner
                    .indexes_memory_usage()
            )
            .unwrap()
    );
}

#[test]
fn storage_gauges_follow_a_secondary_index() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .unwrap();
    let docs = db.get_vector_collection("docs").unwrap();
    docs.upsert(vec![Point::new(
        1,
        vec![1.0, 0.0],
        Some(serde_json::json!({"tag": "a"})),
    )])
    .unwrap();

    let metrics = OperationalMetrics::new();
    db.refresh_storage_gauges(&metrics);
    let before = metrics.index_size_bytes.load(Ordering::Relaxed);

    docs.create_index("tag").unwrap();
    db.refresh_storage_gauges(&metrics);
    assert!(
        metrics.index_size_bytes.load(Ordering::Relaxed) > before,
        "a secondary index has a size: {before} did not grow"
    );
}
