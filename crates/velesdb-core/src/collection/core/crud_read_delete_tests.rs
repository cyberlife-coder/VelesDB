//! Tests for `delete_counted`: a delete reports how many points it removed.

use crate::collection::{Collection, MetadataCollection, VectorCollection};
use crate::distance::DistanceMetric;
use crate::point::Point;
use crate::quantization::StorageMode;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn test_delete_counted_counts_only_ids_that_held_a_point() {
    let dir = tempdir().unwrap();
    let coll = VectorCollection::create(
        dir.path().to_path_buf(),
        "v",
        2,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .unwrap();
    coll.upsert(vec![
        Point::without_payload(1, vec![1.0, 0.0]),
        Point::new(2, vec![0.0, 1.0], Some(json!({"k": "v"}))),
    ])
    .unwrap();

    // A repeated id counts once, a missing one not at all.
    assert_eq!(coll.delete_counted(&[1, 1, 99]).unwrap(), 1);
    assert_eq!(coll.delete_counted(&[1]).unwrap(), 0, "already gone");
    assert_eq!(coll.delete_counted(&[2]).unwrap(), 1);
    assert_eq!(coll.delete_counted(&[]).unwrap(), 0);
    assert!(coll.is_empty());
}

#[test]
fn test_delete_counted_removes_a_payload_only_id_without_counting_it() {
    // A payload-only id of a vector collection (an edge endpoint's payload)
    // is no row of it: `all_ids` does not list it, and deleting it removes
    // the payload but counts nothing. A graph's payload-only node is a row.
    let dir = tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 2, DistanceMetric::Cosine).unwrap();
    coll.store_node_payload(7, &json!({"name": "node"}))
        .unwrap();
    assert!(coll.all_ids().is_empty());

    assert_eq!(coll.delete_counted(&[7]).unwrap(), 0);
    assert!(coll.stored_ids().is_empty(), "the payload is removed");
}

#[test]
fn test_delete_counted_on_a_metadata_collection() {
    let dir = tempdir().unwrap();
    let coll = MetadataCollection::create(dir.path().to_path_buf(), "m").unwrap();
    coll.upsert(vec![
        Point::metadata_only(1, json!({"a": 1})),
        Point::metadata_only(2, json!({"a": 2})),
    ])
    .unwrap();

    assert_eq!(coll.delete_counted(&[2, 3, 2]).unwrap(), 1);
    assert_eq!(coll.len(), 1);
    assert_eq!(coll.delete_counted(&[3]).unwrap(), 0);
}
