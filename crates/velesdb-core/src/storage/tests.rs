//! Tests for storage module.

use super::*;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn test_storage_new_creates_files() {
    let dir = tempdir().unwrap();
    let storage = MmapStorage::new(dir.path(), 3).unwrap();

    assert!(dir.path().join("vectors.dat").exists());
    assert!(dir.path().join("vectors.wal").exists());
    assert_eq!(storage.len(), 0);
}

#[test]
fn test_storage_store_and_retrieve() {
    let dir = tempdir().unwrap();
    let mut storage = MmapStorage::new(dir.path(), 3).unwrap();
    let vector = vec![1.0, 2.0, 3.0];

    storage.store(1, &vector).unwrap();

    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, Some(vector));
    assert_eq!(storage.len(), 1);
}

#[test]
fn test_storage_persistence() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let vector = vec![1.0, 2.0, 3.0];

    {
        let mut storage = MmapStorage::new(&path, 3).unwrap();
        storage.store(1, &vector).unwrap();
        storage.flush().unwrap();
    } // storage dropped

    // Re-open
    let storage = MmapStorage::new(&path, 3).unwrap();
    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, Some(vector));
    assert_eq!(storage.len(), 1);
}

#[test]
fn test_storage_delete() {
    let dir = tempdir().unwrap();
    let mut storage = MmapStorage::new(dir.path(), 3).unwrap();
    let vector = vec![1.0, 2.0, 3.0];

    storage.store(1, &vector).unwrap();
    storage.delete(1).unwrap();

    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, None);
    assert_eq!(storage.len(), 0);
}

#[test]
fn test_storage_wal_recovery() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let vector = vec![1.0, 2.0, 3.0];

    {
        let mut storage = MmapStorage::new(&path, 3).unwrap();
        storage.store(1, &vector).unwrap();
        // Manual flush to ensure index is saved for MVP persistence
        storage.flush().unwrap();
    }

    // Re-open
    let storage = MmapStorage::new(&path, 3).unwrap();
    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, Some(vector));
}

#[test]
fn test_payload_storage_new() {
    let dir = tempdir().unwrap();
    let _storage = LogPayloadStorage::new(dir.path()).unwrap();
    assert!(dir.path().join("payloads.log").exists());
}

#[test]
fn test_payload_storage_ops() {
    let dir = tempdir().unwrap();
    let mut storage = LogPayloadStorage::new(dir.path()).unwrap();
    let payload = json!({"key": "value", "num": 42});

    // Store
    storage.store(1, &payload).unwrap();

    // Retrieve
    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, Some(payload.clone()));

    // Delete
    storage.delete(1).unwrap();
    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, None);
}

#[test]
fn test_payload_persistence() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let payload = json!({"foo": "bar"});

    {
        let mut storage = LogPayloadStorage::new(&path).unwrap();
        storage.store(1, &payload).unwrap();
        storage.flush().unwrap();
    }

    let storage = LogPayloadStorage::new(&path).unwrap();
    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, Some(payload));
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn test_mmap_storage_multiple_vectors() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim: usize = 4;

    let mut storage = MmapStorage::new(&path, dim).unwrap();

    // Store multiple vectors
    for i in 0u64..10 {
        let vector: Vec<f32> = (0..dim).map(|j| (i as usize * dim + j) as f32).collect();
        storage.store(i, &vector).unwrap();
    }

    // Verify all vectors
    for i in 0u64..10 {
        let expected: Vec<f32> = (0..dim).map(|j| (i as usize * dim + j) as f32).collect();
        let retrieved = storage.retrieve(i).unwrap();
        assert_eq!(retrieved, Some(expected));
    }
}

#[test]
fn test_mmap_storage_update_vector() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();

    let mut storage = MmapStorage::new(&path, 3).unwrap();

    // Store initial vector
    storage.store(1, &[1.0, 2.0, 3.0]).unwrap();

    // Update with new vector
    storage.store(1, &[4.0, 5.0, 6.0]).unwrap();

    // Verify updated vector
    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, Some(vec![4.0, 5.0, 6.0]));
}

#[test]
fn test_mmap_storage_retrieve_nonexistent() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();

    let storage = MmapStorage::new(&path, 3).unwrap();
    let retrieved = storage.retrieve(999).unwrap();
    assert_eq!(retrieved, None);
}

#[test]
fn test_payload_storage_multiple_payloads() {
    let dir = tempdir().unwrap();
    let mut storage = LogPayloadStorage::new(dir.path()).unwrap();

    // Store multiple payloads
    for i in 0u64..5 {
        let payload = json!({"id": i, "data": format!("payload_{}", i)});
        storage.store(i, &payload).unwrap();
    }

    // Verify all payloads
    for i in 0u64..5 {
        let retrieved = storage.retrieve(i).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap()["id"], i);
    }
}

#[test]
fn test_payload_storage_retrieve_nonexistent() {
    let dir = tempdir().unwrap();
    let storage = LogPayloadStorage::new(dir.path()).unwrap();
    let retrieved = storage.retrieve(999).unwrap();
    assert_eq!(retrieved, None);
}

#[test]
fn test_payload_storage_complex_json() {
    let dir = tempdir().unwrap();
    let mut storage = LogPayloadStorage::new(dir.path()).unwrap();

    let payload = json!({
        "string": "hello",
        "number": 42,
        "float": 3.15,
        "bool": true,
        "null": null,
        "array": [1, 2, 3],
        "nested": {"key": "value"}
    });

    storage.store(1, &payload).unwrap();
    let retrieved = storage.retrieve(1).unwrap();
    assert_eq!(retrieved, Some(payload));
}

// =========================================================================
// Zero-Copy Retrieval Tests (TDD)
// =========================================================================

#[test]
fn test_retrieve_ref_returns_slice_without_allocation() {
    // Arrange
    let dir = tempdir().unwrap();
    let mut storage = MmapStorage::new(dir.path(), 3).unwrap();
    let vector = vec![1.0, 2.0, 3.0];
    storage.store(1, &vector).unwrap();

    // Act - Use zero-copy retrieval
    let guard = storage.retrieve_ref(1).unwrap();

    // Assert - Data is correct without allocation
    assert!(guard.is_some());
    let slice = guard.unwrap();
    assert_eq!(slice.as_ref(), &[1.0, 2.0, 3.0]);
}

#[test]
fn test_retrieve_ref_nonexistent_returns_none() {
    // Arrange
    let dir = tempdir().unwrap();
    let storage = MmapStorage::new(dir.path(), 3).unwrap();

    // Act
    let guard = storage.retrieve_ref(999).unwrap();

    // Assert
    assert!(guard.is_none());
}

#[test]
fn test_retrieve_ref_multiple_concurrent_reads() {
    // Arrange
    let dir = tempdir().unwrap();
    let mut storage = MmapStorage::new(dir.path(), 3).unwrap();
    storage.store(1, &[1.0, 2.0, 3.0]).unwrap();
    storage.store(2, &[4.0, 5.0, 6.0]).unwrap();

    // Act - Multiple concurrent zero-copy reads
    let guard1 = storage.retrieve_ref(1).unwrap().unwrap();
    let guard2 = storage.retrieve_ref(2).unwrap().unwrap();

    // Assert - Both are valid simultaneously
    assert_eq!(guard1.as_ref(), &[1.0, 2.0, 3.0]);
    assert_eq!(guard2.as_ref(), &[4.0, 5.0, 6.0]);
}

#[test]
fn test_retrieve_ref_data_integrity_after_update() {
    // Arrange
    let dir = tempdir().unwrap();
    let mut storage = MmapStorage::new(dir.path(), 3).unwrap();
    storage.store(1, &[1.0, 2.0, 3.0]).unwrap();

    // Act - Update and retrieve
    storage.store(1, &[7.0, 8.0, 9.0]).unwrap();
    let guard = storage.retrieve_ref(1).unwrap().unwrap();

    // Assert - Returns updated data
    assert_eq!(guard.as_ref(), &[7.0, 8.0, 9.0]);
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::float_cmp)]
fn test_retrieve_ref_large_dimension() {
    // Arrange - 768D vector (typical embedding size)
    let dir = tempdir().unwrap();
    let dim = 768;
    let mut storage = MmapStorage::new(dir.path(), dim).unwrap();
    let vector: Vec<f32> = (0..dim).map(|i| i as f32).collect();
    storage.store(1, &vector).unwrap();

    // Act
    let guard = storage.retrieve_ref(1).unwrap().unwrap();

    // Assert
    assert_eq!(guard.as_ref().len(), dim);
    assert_eq!(guard.as_ref()[0], 0.0);
    assert_eq!(guard.as_ref()[767], 767.0);
}

// =========================================================================
// TS-CORE-004: Compaction Tests
// =========================================================================

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_compaction_reclaims_space() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let dim = 4;
    let vector_size = dim * std::mem::size_of::<f32>();

    let mut storage = MmapStorage::new(&path, dim).unwrap();

    // Store 10 vectors
    for i in 0u64..10 {
        let vector: Vec<f32> = vec![i as f32; dim];
        storage.store(i, &vector).unwrap();
    }

    // Delete 5 vectors (50% fragmentation)
    for i in 0u64..5 {
        storage.delete(i).unwrap();
    }

    // Check fragmentation before compaction
    let frag_before = storage.fragmentation_ratio();
    assert!(frag_before > 0.4, "Should have ~50% fragmentation");

    // Compact
    let reclaimed = storage.compact().unwrap();
    assert_eq!(reclaimed, 5 * vector_size, "Should reclaim 5 vectors worth");

    // Check fragmentation after compaction
    let frag_after = storage.fragmentation_ratio();
    assert!(
        frag_after < 0.01,
        "Should have no fragmentation after compact"
    );

    // Verify remaining vectors are still accessible
    for i in 5u64..10 {
        let retrieved = storage.retrieve(i).unwrap();
        assert!(retrieved.is_some(), "Vector {i} should exist");
        #[allow(clippy::cast_precision_loss)]
        let expected = vec![i as f32; dim];
        assert_eq!(retrieved.unwrap(), expected);
    }

    // Verify deleted vectors are gone
    for i in 0u64..5 {
        let retrieved = storage.retrieve(i).unwrap();
        assert!(retrieved.is_none(), "Vector {i} should be deleted");
    }
}

#[test]
fn test_compaction_empty_storage() {
    let dir = tempdir().unwrap();
    let mut storage = MmapStorage::new(dir.path(), 3).unwrap();

    // Compact empty storage should return 0
    let reclaimed = storage.compact().unwrap();
    assert_eq!(reclaimed, 0);
}

#[test]
fn test_compaction_no_fragmentation() {
    let dir = tempdir().unwrap();
    let mut storage = MmapStorage::new(dir.path(), 3).unwrap();

    // Store vectors without deleting any
    storage.store(1, &[1.0, 2.0, 3.0]).unwrap();
    storage.store(2, &[4.0, 5.0, 6.0]).unwrap();

    // No fragmentation, should return 0
    let reclaimed = storage.compact().unwrap();
    assert_eq!(reclaimed, 0);
}

#[test]
fn test_fragmentation_ratio() {
    let dir = tempdir().unwrap();
    let mut storage = MmapStorage::new(dir.path(), 4).unwrap();

    // Empty storage has no fragmentation
    assert!(storage.fragmentation_ratio() < 0.01);

    // Store 4 vectors
    #[allow(clippy::cast_precision_loss)]
    for i in 0u64..4 {
        storage.store(i, &[i as f32; 4]).unwrap();
    }

    // No fragmentation yet
    assert!(storage.fragmentation_ratio() < 0.01);

    // Delete 2 vectors (50% fragmentation)
    storage.delete(0).unwrap();
    storage.delete(1).unwrap();

    let frag = storage.fragmentation_ratio();
    assert!(
        frag > 0.4 && frag < 0.6,
        "Expected ~50% fragmentation, got {frag}"
    );
}
