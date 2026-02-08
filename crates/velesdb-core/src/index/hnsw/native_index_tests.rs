#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for `native_index` module - Native HNSW index implementation.

#![allow(clippy::useless_vec)]

use super::native_index::*;
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
use tempfile::tempdir;

#[test]
fn test_native_index_new() {
    let index = NativeHnswIndex::new(64, DistanceMetric::Euclidean);
    assert_eq!(index.dimension(), 64);
    assert_eq!(index.metric(), DistanceMetric::Euclidean);
    assert!(index.is_empty());
}

#[test]
fn test_native_index_insert_search() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean);

    for i in 0..50 {
        let vec: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect();
        index.insert(i, &vec);
    }

    assert_eq!(index.len(), 50);

    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.01).collect();
    let results = index.search(&query, 5);

    assert!(!results.is_empty());
    assert!(results.len() <= 5);
    assert_eq!(results[0].0, 0);
}

#[test]
fn test_native_index_batch_insert() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean);

    let items: Vec<(u64, Vec<f32>)> = (0..50).map(|i| (i, vec![i as f32 * 0.01; 32])).collect();

    index.insert_batch(&items);

    assert_eq!(index.len(), 50);
}

#[test]
fn test_native_index_persistence() {
    let dir = tempdir().unwrap();

    let index = NativeHnswIndex::new(32, DistanceMetric::Cosine);
    for i in 0..30 {
        index.insert(i, &vec![i as f32 * 0.1; 32]);
    }

    index.save(dir.path()).unwrap();

    let loaded = NativeHnswIndex::load(dir.path(), 32, DistanceMetric::Cosine).unwrap();

    assert_eq!(loaded.dimension(), 32);
    assert_eq!(loaded.metric(), DistanceMetric::Cosine);
    assert_eq!(loaded.len(), 30);

    let results = loaded.search(&vec![0.0; 32], 5);
    assert!(!results.is_empty());
}

#[test]
fn test_native_index_delete() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean);
    index.insert(1, &vec![0.1; 32]);
    index.insert(2, &vec![0.2; 32]);

    assert!(index.remove(1));
    assert!(!index.remove(999));
}

#[test]
fn test_native_index_vector_index_trait() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean);

    <NativeHnswIndex as VectorIndex>::insert(&index, 1, &vec![0.1; 32]);
    assert_eq!(<NativeHnswIndex as VectorIndex>::len(&index), 1);

    let results = <NativeHnswIndex as VectorIndex>::search(&index, &vec![0.1; 32], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 1);
}

#[test]
fn test_native_index_brute_force_search() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean);

    for i in 0..20u64 {
        let vec: Vec<f32> = (0..32u64).map(|j| (i * 32 + j) as f32 * 0.001).collect();
        index.insert(i, &vec);
    }

    let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.001).collect();
    let results = index.brute_force_search_parallel(&query, 5);

    assert_eq!(results.len(), 5);
    assert_eq!(results[0].0, 0);
    for i in 1..results.len() {
        assert!(results[i].1 >= results[i - 1].1, "Results not sorted");
    }
}

#[test]
fn test_native_index_brute_force_empty() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean);
    let query = vec![0.0; 32];
    let results = index.brute_force_search_parallel(&query, 5);
    assert!(results.is_empty());
}

#[test]
fn test_native_index_brute_force_k_larger_than_size() {
    let index = NativeHnswIndex::new(32, DistanceMetric::Euclidean);
    index.insert(1, &vec![0.1; 32]);
    index.insert(2, &vec![0.2; 32]);

    let results = index.brute_force_search_parallel(&vec![0.0; 32], 10);
    assert_eq!(results.len(), 2);
}
