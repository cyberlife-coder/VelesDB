//! Tests for `vector_store` module

use super::vector_store::*;

#[test]
fn test_vector_store_new() {
    let store = VectorStore::new(768, 1000);
    assert_eq!(store.dimension(), 768);
    assert_eq!(store.len(), 0);
    assert!(store.is_empty());
}

#[test]
fn test_vector_store_insert_and_get() {
    let store = VectorStore::new(4, 10);
    let vec1 = vec![1.0, 2.0, 3.0, 4.0];
    let vec2 = vec![5.0, 6.0, 7.0, 8.0];

    let idx1 = store.insert(&vec1);
    let idx2 = store.insert(&vec2);

    assert_eq!(idx1, 0);
    assert_eq!(idx2, 1);
    assert_eq!(store.len(), 2);

    assert_eq!(store.get(idx1), Some(vec1));
    assert_eq!(store.get(idx2), Some(vec2));
}

#[test]
fn test_vector_store_get_slice() {
    let store = VectorStore::new(3, 10);
    let vec1 = vec![1.0, 2.0, 3.0];

    let idx = store.insert(&vec1);
    let slice_ref = store.get_slice(idx).unwrap();

    assert_eq!(slice_ref.as_slice(), &[1.0, 2.0, 3.0]);
    assert_eq!(&*slice_ref, &[1.0, 2.0, 3.0]); // Test Deref
}

#[test]
fn test_vector_store_update() {
    let store = VectorStore::new(3, 10);
    let vec1 = vec![1.0, 2.0, 3.0];
    let vec2 = vec![4.0, 5.0, 6.0];

    let idx = store.insert(&vec1);
    assert!(store.update(idx, &vec2));
    assert_eq!(store.get(idx), Some(vec2));
}

#[test]
fn test_vector_store_remove_and_reuse() {
    let store = VectorStore::new(2, 10);
    let vec1 = vec![1.0, 2.0];
    let vec2 = vec![3.0, 4.0];
    let vec3 = vec![5.0, 6.0];

    let idx1 = store.insert(&vec1);
    let idx2 = store.insert(&vec2);

    // Remove first vector
    assert!(store.remove(idx1));

    // Insert new vector should reuse slot
    let idx3 = store.insert(&vec3);
    assert_eq!(idx3, idx1); // Should reuse slot 0

    assert_eq!(store.get(idx2), Some(vec2));
    assert_eq!(store.get(idx3), Some(vec3));
}

#[test]
fn test_vector_store_invalid_index() {
    let store = VectorStore::new(3, 10);
    assert!(store.get(0).is_none());
    assert!(store.get(100).is_none());
    assert!(!store.remove(100));
    assert!(!store.update(100, &[1.0, 2.0, 3.0]));
}

#[test]
#[should_panic(expected = "Vector dimension mismatch")]
fn test_vector_store_dimension_mismatch_insert() {
    let store = VectorStore::new(3, 10);
    store.insert(&[1.0, 2.0]); // Wrong dimension
}

#[test]
#[should_panic(expected = "Vector dimension mismatch")]
fn test_vector_store_dimension_mismatch_update() {
    let store = VectorStore::new(3, 10);
    let idx = store.insert(&[1.0, 2.0, 3.0]);
    store.update(idx, &[1.0, 2.0]); // Wrong dimension
}

#[test]
fn test_vector_store_memory_usage() {
    let store = VectorStore::new(768, 1000);
    // Pre-allocated capacity should be 768 * 1000 * 4 bytes = ~3MB
    let usage = store.memory_usage();
    assert!(usage >= 768 * 1000 * 4);
}

#[test]
fn test_vector_store_prefetch() {
    let store = VectorStore::new(4, 10);
    let idx = store.insert(&[1.0, 2.0, 3.0, 4.0]);

    // Prefetch should not panic
    store.prefetch(idx);
    store.prefetch(100); // Invalid index should be handled gracefully
}
