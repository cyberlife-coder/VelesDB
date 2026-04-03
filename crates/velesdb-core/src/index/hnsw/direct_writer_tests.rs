//! Tests for `DirectVectorWriter`.

use super::direct_writer::DirectVectorWriter;
use super::HnswIndex;
use crate::distance::DistanceMetric;

/// Creates a test `HnswIndex` with the given dimension and vector storage enabled.
fn make_index(dim: usize) -> HnswIndex {
    HnswIndex::new(dim, DistanceMetric::Cosine).expect("test index creation")
}

/// Creates a test `HnswIndex` with vector storage disabled.
fn make_index_no_storage(dim: usize) -> HnswIndex {
    HnswIndex::new_fast_insert(dim, DistanceMetric::Cosine).expect("test index creation")
}

#[test]
fn test_write_batch_direct_empty() {
    let index = make_index(4);
    let writer = DirectVectorWriter::new(&index);
    let results = writer.write_batch_direct(&[]).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_write_batch_direct_single_vector() {
    let index = make_index(4);
    let writer = DirectVectorWriter::new(&index);
    let vec = [1.0_f32, 2.0, 3.0, 4.0];
    let results = writer.write_batch_direct(&[(1, &vec)]).unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].old_idx.is_none());

    // Verify mapping exists
    assert!(index.mappings.get_idx(1).is_some());

    // Verify vector is in ContiguousVectors (via inner)
    let stored = index
        .inner
        .read()
        .with_contiguous_vectors_read(|cv| cv.get(results[0].idx).map(|s| s.to_vec()));
    assert_eq!(stored, Some(vec.to_vec()));

    // Verify ShardedVectors is empty (bypass)
    assert!(index.vectors.is_empty());
}

#[test]
fn test_write_batch_direct_multiple_vectors() {
    let index = make_index(3);
    let writer = DirectVectorWriter::new(&index);
    let v1 = [1.0_f32, 0.0, 0.0];
    let v2 = [0.0_f32, 1.0, 0.0];
    let v3 = [0.0_f32, 0.0, 1.0];
    let batch: Vec<(u64, &[f32])> = vec![(10, &v1), (20, &v2), (30, &v3)];

    let results = writer.write_batch_direct(&batch).unwrap();
    assert_eq!(results.len(), 3);

    // All mappings registered
    assert!(index.mappings.get_idx(10).is_some());
    assert!(index.mappings.get_idx(20).is_some());
    assert!(index.mappings.get_idx(30).is_some());

    // ShardedVectors still empty
    assert!(index.vectors.is_empty());
}

#[test]
fn test_sync_to_sharded_populates_sharded_vectors() {
    let index = make_index(3);
    let writer = DirectVectorWriter::new(&index);
    let v1 = [1.0_f32, 2.0, 3.0];
    let v2 = [4.0_f32, 5.0, 6.0];
    let batch: Vec<(u64, &[f32])> = vec![(1, &v1), (2, &v2)];

    let results = writer.write_batch_direct(&batch).unwrap();
    assert!(index.vectors.is_empty());

    writer.sync_to_sharded(&results).unwrap();

    // Now ShardedVectors should have the vectors
    assert_eq!(index.vectors.len(), 2);
    let stored1 = index.vectors.get(results[0].idx).unwrap();
    assert_eq!(stored1, v1.to_vec());
    let stored2 = index.vectors.get(results[1].idx).unwrap();
    assert_eq!(stored2, v2.to_vec());
}

#[test]
fn test_upsert_deduplication() {
    let index = make_index(2);
    let writer = DirectVectorWriter::new(&index);
    let v1 = [1.0_f32, 0.0];
    let v2 = [0.0_f32, 1.0];

    // Insert ID=1 twice — second should replace first
    let r1 = writer.write_batch_direct(&[(1, &v1)]).unwrap();
    let r2 = writer.write_batch_direct(&[(1, &v2)]).unwrap();

    assert!(r1[0].old_idx.is_none());
    assert!(r2[0].old_idx.is_some());
    assert_eq!(r2[0].old_idx, Some(r1[0].idx));

    // Only one mapping for ID=1
    assert_eq!(index.mappings.len(), 1);
    let current_idx = index.mappings.get_idx(1).unwrap();
    assert_eq!(current_idx, r2[0].idx);
}

#[test]
fn test_dimension_mismatch_returns_error() {
    let index = make_index(4);
    let writer = DirectVectorWriter::new(&index);
    let wrong_dim = [1.0_f32, 2.0]; // dim=2, expected=4

    let result = writer.write_batch_direct(&[(1, &wrong_dim)]);
    assert!(result.is_err());

    // State unchanged
    assert!(index.mappings.is_empty());
}

#[test]
fn test_storage_bypass_when_disabled() {
    let index = make_index_no_storage(3);
    let writer = DirectVectorWriter::new(&index);
    let v = [1.0_f32, 2.0, 3.0];

    let results = writer.write_batch_direct(&[(1, &v)]).unwrap();
    assert_eq!(results.len(), 1);

    // Mapping exists
    assert!(index.mappings.get_idx(1).is_some());

    // ShardedVectors empty (storage disabled)
    assert!(index.vectors.is_empty());

    // sync_to_sharded is a no-op
    writer.sync_to_sharded(&results).unwrap();
    assert!(index.vectors.is_empty());
}

#[test]
fn test_sync_to_sharded_empty_results() {
    let index = make_index(3);
    let writer = DirectVectorWriter::new(&index);
    writer.sync_to_sharded(&[]).unwrap();
    assert!(index.vectors.is_empty());
}
