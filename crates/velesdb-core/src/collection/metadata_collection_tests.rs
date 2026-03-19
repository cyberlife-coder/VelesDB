//! Tests for [`MetadataCollection`] public API.

use crate::collection::MetadataCollection;
use crate::error::Error;
use crate::point::Point;
use serde_json::json;
use tempfile::tempdir;

// =========================================================================
// Lifecycle: create + name + len + is_empty
// =========================================================================

#[test]
fn test_create_name_len_empty() {
    let dir = tempdir().unwrap();
    let coll = MetadataCollection::create(dir.path().to_path_buf(), "items").unwrap();

    assert_eq!(coll.name(), "items");
    assert_eq!(coll.len(), 0);
    assert!(coll.is_empty());
}

// =========================================================================
// Upsert metadata points and get
// =========================================================================

#[test]
fn test_upsert_and_get() {
    let dir = tempdir().unwrap();
    let coll = MetadataCollection::create(dir.path().to_path_buf(), "products").unwrap();

    coll.upsert(vec![
        Point::metadata_only(1, json!({"name": "Widget", "price": 9.99})),
        Point::metadata_only(2, json!({"name": "Gadget", "price": 19.99})),
    ])
    .unwrap();

    assert_eq!(coll.len(), 2);
    assert!(!coll.is_empty());

    let results = coll.get(&[1, 2, 999]);
    assert!(results[0].is_some());
    assert_eq!(results[0].as_ref().unwrap().id, 1);
    assert!(results[1].is_some());
    assert_eq!(results[1].as_ref().unwrap().id, 2);
    assert!(results[2].is_none());
}

// =========================================================================
// Upsert with vector returns VectorNotAllowed
// =========================================================================

#[test]
fn test_upsert_with_vector_returns_error() {
    let dir = tempdir().unwrap();
    let coll = MetadataCollection::create(dir.path().to_path_buf(), "meta_only").unwrap();

    let point_with_vector = Point::new(1, vec![1.0, 2.0, 3.0], Some(json!({"k": "v"})));
    let err = coll.upsert(vec![point_with_vector]).unwrap_err();

    assert!(
        matches!(err, Error::VectorNotAllowed(_)),
        "expected VectorNotAllowed, got: {err:?}"
    );
}

// =========================================================================
// Delete points
// =========================================================================

#[test]
fn test_delete_points() {
    let dir = tempdir().unwrap();
    let coll = MetadataCollection::create(dir.path().to_path_buf(), "del_test").unwrap();

    coll.upsert(vec![
        Point::metadata_only(10, json!({"a": 1})),
        Point::metadata_only(20, json!({"b": 2})),
        Point::metadata_only(30, json!({"c": 3})),
    ])
    .unwrap();

    assert_eq!(coll.len(), 3);

    coll.delete(&[10, 30]).unwrap();
    assert_eq!(coll.len(), 1);

    let remaining = coll.get(&[10, 20, 30]);
    assert!(remaining[0].is_none());
    assert!(remaining[1].is_some());
    assert!(remaining[2].is_none());
}

// =========================================================================
// text_search returns empty for metadata-only (no vector_storage entries)
// =========================================================================

#[test]
fn test_text_search_returns_empty_for_metadata_only() {
    let dir = tempdir().unwrap();
    let coll = MetadataCollection::create(dir.path().to_path_buf(), "docs").unwrap();

    coll.upsert(vec![
        Point::metadata_only(1, json!({"title": "Rust programming language"})),
        Point::metadata_only(2, json!({"title": "Python programming guide"})),
    ])
    .unwrap();

    // text_search delegates to BM25 + hydrate_point; hydrate_point requires
    // vector_storage entries which metadata-only points lack, so the result
    // set is empty.  The call itself must not error.
    let results = coll.text_search("programming", 10).unwrap();
    assert!(
        results.is_empty(),
        "text_search returns empty for metadata-only points (no vector storage)"
    );
}

// =========================================================================
// execute_query_str with simple VelesQL
// =========================================================================

#[test]
fn test_execute_query_str_select_all() {
    let dir = tempdir().unwrap();
    let coll = MetadataCollection::create(dir.path().to_path_buf(), "velesql_test").unwrap();

    coll.upsert(vec![
        Point::metadata_only(1, json!({"name": "alpha"})),
        Point::metadata_only(2, json!({"name": "beta"})),
        Point::metadata_only(3, json!({"name": "gamma"})),
    ])
    .unwrap();

    let results = coll
        .execute_query_str(
            "SELECT * FROM velesql_test LIMIT 10",
            &std::collections::HashMap::new(),
        )
        .unwrap();

    assert_eq!(results.len(), 3);
}

// =========================================================================
// all_ids
// =========================================================================

#[test]
fn test_all_ids() {
    let dir = tempdir().unwrap();
    let coll = MetadataCollection::create(dir.path().to_path_buf(), "id_test").unwrap();

    coll.upsert(vec![
        Point::metadata_only(5, json!({"x": 1})),
        Point::metadata_only(10, json!({"x": 2})),
        Point::metadata_only(15, json!({"x": 3})),
    ])
    .unwrap();

    let mut ids = coll.all_ids();
    ids.sort_unstable();
    assert_eq!(ids, vec![5, 10, 15]);
}

// =========================================================================
// Persistence: open after create
// =========================================================================

#[test]
fn test_open_after_create_persistence() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();

    // Create and populate, then flush.
    {
        let coll = MetadataCollection::create(path.clone(), "persist").unwrap();
        coll.upsert(vec![
            Point::metadata_only(1, json!({"key": "value1"})),
            Point::metadata_only(2, json!({"key": "value2"})),
        ])
        .unwrap();
        coll.flush().unwrap();
    }

    // Re-open from disk and verify data survived.
    {
        let coll = MetadataCollection::open(path).unwrap();
        assert_eq!(coll.name(), "persist");
        assert_eq!(coll.len(), 2);

        let results = coll.get(&[1, 2]);
        assert!(results[0].is_some());
        assert!(results[1].is_some());
    }
}
