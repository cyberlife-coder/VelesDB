#![allow(deprecated)] // Tests use legacy Collection via get_collection().

use super::*;
use crate::point::Point;
use crate::{CollectionType, DistanceMetric};
use tempfile::tempdir;

// =========================================================================
// Create + Get + Delete lifecycle
// =========================================================================

#[test]
fn test_create_get_delete_lifecycle() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Create
    db.create_collection("lifecycle", 128, DistanceMetric::Cosine)
        .unwrap();
    assert_eq!(db.list_collections(), vec!["lifecycle"]);

    // Get
    let coll = db.get_collection("lifecycle");
    assert!(coll.is_some());
    assert_eq!(coll.unwrap().config().dimension, 128);

    // Delete
    db.delete_collection("lifecycle").unwrap();
    assert!(db.list_collections().is_empty());
    assert!(db.get_collection("lifecycle").is_none());
}

// =========================================================================
// Duplicate creation
// =========================================================================

#[test]
fn test_create_duplicate_returns_collection_exists() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("dup", 64, DistanceMetric::Euclidean)
        .unwrap();

    let err = db
        .create_collection("dup", 64, DistanceMetric::Euclidean)
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

#[test]
fn test_create_vector_duplicate_returns_error() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_vector_collection("vec_dup", 32, DistanceMetric::Cosine)
        .unwrap();

    let err = db
        .create_vector_collection("vec_dup", 32, DistanceMetric::Cosine)
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

#[test]
fn test_create_metadata_duplicate_returns_error() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_metadata_collection("meta_dup").unwrap();

    let err = db.create_metadata_collection("meta_dup").unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

// =========================================================================
// Nonexistent lookups
// =========================================================================

#[test]
fn test_get_nonexistent_returns_none() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    assert!(db.get_collection("nope").is_none());
    assert!(db.get_vector_collection("nope").is_none());
    assert!(db.get_graph_collection("nope").is_none());
    assert!(db.get_metadata_collection("nope").is_none());
}

#[test]
fn test_delete_nonexistent_returns_not_found() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let err = db.delete_collection("absent").unwrap_err();
    assert!(matches!(err, crate::Error::CollectionNotFound(_)));
}

// =========================================================================
// Multi-collection isolation
// =========================================================================

#[test]
fn test_multi_collection_data_isolation() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("coll_a", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("coll_b", 4, DistanceMetric::Euclidean)
        .unwrap();

    // Insert into coll_a only.
    let coll_a = db.get_collection("coll_a").unwrap();
    coll_a
        .upsert(vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"source": "a"})),
        )])
        .unwrap();

    // coll_b must remain empty.
    let coll_b = db.get_collection("coll_b").unwrap();
    assert!(coll_b.get(&[1]).into_iter().flatten().next().is_none());
}

#[test]
fn test_list_collections_returns_sorted_names() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("zebra", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("alpha", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("middle", 4, DistanceMetric::Cosine)
        .unwrap();

    let names = db.list_collections();
    assert_eq!(names, vec!["alpha", "middle", "zebra"]);
}

// =========================================================================
// Typed collection creation via create_collection_typed
// =========================================================================

#[test]
fn test_create_collection_typed_metadata_only() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    assert!(db.get_metadata_collection("products").is_some());
    assert_eq!(db.list_collections(), vec!["products"]);
}

#[test]
fn test_create_collection_typed_vector() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let kind = CollectionType::Vector {
        dimension: 128,
        metric: DistanceMetric::DotProduct,
        storage_mode: crate::StorageMode::Full,
    };
    db.create_collection_typed("embeddings", &kind).unwrap();

    let vc = db.get_vector_collection("embeddings").unwrap();
    assert_eq!(vc.inner.config().dimension, 128);
    assert_eq!(vc.inner.config().metric, DistanceMetric::DotProduct);
}

// =========================================================================
// Delete cleans up disk directory
// =========================================================================

#[test]
fn test_delete_collection_removes_disk_directory() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("cleanup", 4, DistanceMetric::Cosine)
        .unwrap();
    let coll_path = dir.path().join("cleanup");
    assert!(coll_path.exists(), "collection dir should exist on disk");

    db.delete_collection("cleanup").unwrap();
    assert!(
        !coll_path.exists(),
        "collection dir should be removed after delete"
    );
}

// =========================================================================
// Cross-type name collision
// =========================================================================

#[test]
fn test_name_collision_across_collection_types() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_metadata_collection("shared_name").unwrap();

    // Attempting to create a vector collection with the same name must fail.
    let err = db
        .create_vector_collection("shared_name", 64, DistanceMetric::Cosine)
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}
