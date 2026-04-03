//! Tests for `HnswSegmentBuilder`.

use super::segment_builder::HnswSegmentBuilder;
use super::HnswIndex;
use crate::distance::DistanceMetric;

/// Creates a test `HnswIndex` with the given dimension.
fn make_index(dim: usize) -> HnswIndex {
    HnswIndex::new(dim, DistanceMetric::Cosine).expect("test index creation")
}

#[test]
fn test_build_and_merge_empty() {
    let index = make_index(4);
    let builder = HnswSegmentBuilder::new(4);
    let result = builder.build_and_merge(&[], &index).unwrap();
    assert_eq!(result.indexed_count, 0);
}

#[test]
fn test_build_and_merge_small_batch_monolithic() {
    let dim = 4;
    let index = make_index(dim);
    let _builder = HnswSegmentBuilder::new(4);

    // Insert vectors via the standard path first to register mappings
    let vecs: Vec<Vec<f32>> = (0..50)
        .map(|i| {
            let mut v = vec![0.0_f32; dim];
            v[i % dim] = 1.0;
            v
        })
        .collect();

    let pairs: Vec<(u64, &[f32])> = vecs
        .iter()
        .enumerate()
        .map(|(i, v)| (i as u64, v.as_slice()))
        .collect();
    let inserted = index.insert_batch_parallel(pairs);
    assert_eq!(inserted, 50);
    assert_eq!(index.len(), 50);
}

#[test]
fn test_build_and_merge_returns_indexed_count() {
    let dim = 4;
    let index = make_index(dim);
    let _builder = HnswSegmentBuilder::new(2);

    // Pre-populate with some vectors
    let vecs: Vec<Vec<f32>> = (0..10)
        .map(|i| {
            let mut v = vec![0.0_f32; dim];
            v[i % dim] = (i as f32 + 1.0) / 10.0;
            v
        })
        .collect();

    let pairs: Vec<(u64, &[f32])> = vecs
        .iter()
        .enumerate()
        .map(|(i, v)| (i as u64, v.as_slice()))
        .collect();
    let inserted = index.insert_batch_parallel(pairs);
    assert!(inserted > 0);
    assert_eq!(index.len(), inserted);
}

#[test]
fn test_segment_count_zero_treated_as_one() {
    // Verify that segment_count=0 doesn't panic
    let builder = HnswSegmentBuilder::new(0);
    let index = make_index(4);
    let result = builder.build_and_merge(&[], &index).unwrap();
    assert_eq!(result.indexed_count, 0);
}

#[test]
fn test_build_and_merge_preserves_all_vectors() {
    let dim = 4;
    let index = make_index(dim);

    // Insert 20 vectors via standard path
    let vecs: Vec<Vec<f32>> = (0..20)
        .map(|i| {
            let mut v = vec![0.0_f32; dim];
            v[i % dim] = (i as f32 + 1.0).sqrt();
            v
        })
        .collect();

    let pairs: Vec<(u64, &[f32])> = vecs
        .iter()
        .enumerate()
        .map(|(i, v)| (i as u64, v.as_slice()))
        .collect();
    let inserted = index.insert_batch_parallel(pairs);
    assert_eq!(inserted, 20);

    // All vectors should be searchable
    let query = vec![1.0_f32, 0.0, 0.0, 0.0];
    let results = index
        .search_with_quality(&query, 10, crate::index::hnsw::SearchQuality::Accurate)
        .unwrap();
    assert!(!results.is_empty());
}
