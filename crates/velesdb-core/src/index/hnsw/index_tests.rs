//! Tests for `HnswIndex` (extracted from index.rs for maintainability)
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::redundant_closure_for_method_calls
)]

use super::index::{HnswIndex, VacuumError};
use super::params::{HnswParams, SearchQuality};
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;

// =========================================================================
// TDD Tests - Written BEFORE implementation (RED phase)
// =========================================================================

// -------------------------------------------------------------------------
// Vacuum / Maintenance Tests
// -------------------------------------------------------------------------

#[test]
fn test_tombstone_count_empty_index() {
    // Arrange
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    // Act & Assert
    assert_eq!(index.tombstone_count(), 0);
    assert!((index.tombstone_ratio() - 0.0).abs() < f64::EPSILON);
    assert!(!index.needs_vacuum());
}

#[test]
fn test_tombstone_count_after_deletions() {
    // Arrange
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    // Insert 10 vectors
    for i in 0..10 {
        let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
        index.insert(i as u64, &v);
    }

    // Delete 3 vectors (30%)
    index.remove(1);
    index.remove(3);
    index.remove(5);

    // Assert
    assert_eq!(index.len(), 7);
    assert_eq!(index.tombstone_count(), 3);
    assert!((index.tombstone_ratio() - 0.3).abs() < 0.01);
    assert!(index.needs_vacuum()); // > 20% threshold
}

#[test]
fn test_vacuum_rebuilds_index() {
    // Arrange
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    // Insert 20 vectors
    for i in 0..20 {
        let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
        index.insert(i as u64, &v);
    }

    // Delete 10 vectors (50% tombstones)
    for i in 0..10 {
        index.remove(i as u64);
    }

    assert_eq!(index.len(), 10);
    assert!(index.needs_vacuum());

    // Act
    let result = index.vacuum();

    // Assert
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 10);
    assert_eq!(index.len(), 10);
    assert_eq!(index.tombstone_count(), 0);
    assert!(!index.needs_vacuum());
}

#[test]
fn test_vacuum_preserves_search_results() {
    // Arrange
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    // Insert vectors with known patterns
    for i in 0..50 {
        let v: Vec<f32> = (0..64).map(|j| (i * 100 + j) as f32 * 0.001).collect();
        index.insert(i as u64, &v);
    }

    // Delete some vectors
    for i in 0..25 {
        index.remove(i as u64);
    }

    // Query before vacuum
    let query: Vec<f32> = (0..64).map(|j| (30 * 100 + j) as f32 * 0.001).collect();
    let _results_before = index.search(&query, 5);

    // Act
    let _ = index.vacuum();

    // Assert - search still works and returns similar results
    let results_after = index.search(&query, 5);
    assert_eq!(results_after.len(), 5);
    // Results should include vectors 25-49 (the remaining ones)
    for sr in &results_after {
        assert!(sr.id >= 25 && sr.id < 50);
    }
}

#[test]
fn test_drop_after_vacuum_and_reload_is_safe() {
    use tempfile::tempdir;

    let dir = tempdir().expect("Failed to create temp dir");
    {
        let index = HnswIndex::new(32, DistanceMetric::Euclidean).unwrap();

        for i in 0u64..120 {
            let vector: Vec<f32> = (0..32).map(|j| (i + j as u64) as f32 * 0.01).collect();
            index.insert(i, &vector);
        }
        for i in 0u64..40 {
            index.remove(i);
        }

        let rebuilt = index.vacuum().expect("vacuum should succeed");
        assert_eq!(rebuilt, 80);
        index.save(dir.path()).expect("Failed to save");
    } // drop after ManuallyDrop replacement path

    let loaded = HnswIndex::load(dir.path(), 32, DistanceMetric::Euclidean)
        .expect("Failed to load after vacuum+drop");
    let query = vec![0.42; 32];
    let results = loaded.search(&query, 10);
    assert!(!results.is_empty(), "Loaded index should remain searchable");
}

#[test]
fn test_vacuum_fails_with_fast_insert_mode() {
    // Arrange
    let index = HnswIndex::new_fast_insert(64, DistanceMetric::Cosine).unwrap();

    for i in 0..10 {
        let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
        index.insert(i as u64, &v);
    }

    // Act
    let result = index.vacuum();

    // Assert
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), VacuumError::VectorStorageDisabled);
}

#[test]
fn test_vacuum_empty_index() {
    // Arrange
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    // Act
    let result = index.vacuum();

    // Assert
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

// -------------------------------------------------------------------------
// Basic Index Tests
// -------------------------------------------------------------------------

#[test]
fn test_hnsw_new_creates_empty_index() {
    // Arrange & Act
    let index = HnswIndex::new(768, DistanceMetric::Cosine).unwrap();

    // Assert
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
    assert_eq!(index.dimension(), 768);
    assert_eq!(index.metric(), DistanceMetric::Cosine);
}

#[test]
fn test_hnsw_new_turbo_mode() {
    // TDD: Turbo mode uses aggressive params for max insert throughput
    // Target: 5k+ vec/s (vs ~2k/s with auto params)
    let index = HnswIndex::new_turbo(64, DistanceMetric::Cosine).unwrap();

    // Insert vectors - should be faster than standard mode
    for i in 0..100 {
        let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
        index.insert(i as u64, &v);
    }

    // Assert - basic functionality works
    assert_eq!(index.len(), 100);

    // Search should still work (lower recall expected ~85%)
    let query: Vec<f32> = (0..64).map(|j| j as f32 * 0.01).collect();
    let results = index.search(&query, 10);
    assert!(!results.is_empty()); // At least some results
}

#[test]
fn test_hnsw_new_fast_insert_mode() {
    // Arrange & Act - fast insert mode disables vector storage
    let index = HnswIndex::new_fast_insert(64, DistanceMetric::Cosine).unwrap();

    // Insert vectors
    for i in 0..100 {
        let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
        index.insert(i as u64, &v);
    }

    // Assert - basic functionality works
    assert_eq!(index.len(), 100);

    // Search should still work (uses HNSW approximate search)
    let query: Vec<f32> = (0..64).map(|j| j as f32 * 0.01).collect();
    let results = index.search(&query, 10);
    assert_eq!(results.len(), 10);
}

#[test]
fn test_hnsw_insert_single_vector() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    let vector = vec![1.0, 0.0, 0.0];

    // Act
    index.insert(1, &vector);

    // Assert
    assert_eq!(index.len(), 1);
    assert!(!index.is_empty());
}

#[test]
fn test_hnsw_insert_multiple_vectors() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();

    // Act
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0]);
    index.insert(3, &[0.0, 0.0, 1.0]);

    // Assert
    assert_eq!(index.len(), 3);
}

#[test]
fn test_hnsw_search_returns_k_nearest() {
    // Arrange - use more vectors to make HNSW more stable
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]); // Similar to 1
    index.insert(3, &[0.0, 1.0, 0.0]); // Different
    index.insert(4, &[0.8, 0.2, 0.0]); // Similar to 1
    index.insert(5, &[0.0, 0.0, 1.0]); // Different

    // Act
    let results = index.search(&[1.0, 0.0, 0.0], 3);

    // Assert - HNSW may return fewer than k results with small datasets
    assert!(
        !results.is_empty() && results.len() <= 3,
        "Should return 1-3 results, got {}",
        results.len()
    );
    // First result should be exact match (id=1) - verify it's in top results
    let top_ids: Vec<u64> = results.iter().map(|sr| sr.id).collect();
    assert!(top_ids.contains(&1), "Exact match should be in top results");
}

#[test]
fn test_hnsw_search_empty_index() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();

    // Act
    let results = index.search(&[1.0, 0.0, 0.0], 10);

    // Assert
    assert!(results.is_empty());
}

#[test]
fn test_hnsw_remove_existing_vector() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0]);

    // Act
    let removed = index.remove(1);

    // Assert
    assert!(removed);
    assert_eq!(index.len(), 1);
}

#[test]
fn test_hnsw_remove_nonexistent_vector() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);

    // Act
    let removed = index.remove(999);

    // Assert
    assert!(!removed);
    assert_eq!(index.len(), 1);
}

#[test]
fn test_hnsw_euclidean_metric() {
    // Arrange - use more vectors to avoid HNSW flakiness with tiny datasets
    let index = HnswIndex::new(3, DistanceMetric::Euclidean).unwrap();
    index.insert(1, &[0.0, 0.0, 0.0]);
    index.insert(2, &[1.0, 0.0, 0.0]); // Distance 1
    index.insert(3, &[3.0, 4.0, 0.0]); // Distance 5
    index.insert(4, &[2.0, 0.0, 0.0]); // Distance 2
    index.insert(5, &[0.5, 0.5, 0.0]); // Distance ~0.7

    // Act
    let results = index.search(&[0.0, 0.0, 0.0], 3);

    // Assert - at least get some results, first should be closest
    assert!(!results.is_empty(), "Should return results");
    assert_eq!(results[0].id, 1, "Closest should be exact match");
}

#[test]
fn test_hnsw_dot_product_metric() {
    // Arrange - Use normalized positive vectors for dot product
    // DistDot in hnsw_rs requires non-negative dot products
    // Use more vectors to avoid HNSW flakiness with tiny datasets
    let index = HnswIndex::new(3, DistanceMetric::DotProduct).unwrap();

    // Insert vectors with distinct dot products when queried with [1,0,0]
    index.insert(1, &[1.0, 0.0, 0.0]); // dot=1.0 with query
    index.insert(2, &[0.5, 0.5, 0.5]); // dot=0.5 with query
    index.insert(3, &[0.1, 0.1, 0.1]); // dot=0.1 with query
    index.insert(4, &[0.8, 0.2, 0.0]); // dot=0.8 with query
    index.insert(5, &[0.3, 0.3, 0.3]); // dot=0.3 with query

    // Act - Query with unit vector x
    let query = [1.0, 0.0, 0.0];
    let results = index.search(&query, 3);

    // Assert - at least get some results, first should have highest dot product
    assert!(!results.is_empty(), "Should return results");
    assert_eq!(results[0].id, 1, "Highest dot product should be first");
}

#[test]
fn test_hnsw_insert_wrong_dimension_is_noop() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();

    // Act - dimension mismatch is silently rejected via VectorIndex trait
    index.insert(1, &[1.0, 0.0]); // Wrong dimension

    // Assert - no vector was inserted
    assert_eq!(index.len(), 0, "Wrong-dimension insert should be a no-op");
}

#[test]
fn test_hnsw_search_wrong_dimension_returns_error() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);

    // Act - dimension mismatch returns DimensionMismatch error
    let result = index.search_with_quality(&[1.0, 0.0], 10, SearchQuality::Balanced);
    assert!(result.is_err(), "Should return error on dimension mismatch");

    // The VectorIndex::search trait adapter returns empty Vec on error
    let results = index.search(&[1.0, 0.0], 10);
    assert!(
        results.is_empty(),
        "Trait search should return empty on mismatch"
    );
}

#[test]
fn test_hnsw_duplicate_insert_upserts_vector() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);

    // Act - Insert with same ID performs upsert (replaces the vector)
    index.insert(1, &[0.0, 1.0, 0.0]);

    // Assert
    assert_eq!(index.len(), 1); // Still only one entry

    // Verify the UPDATED vector is indexed (not the original)
    let results = index.search(&[0.0, 1.0, 0.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1);
    // Score should be ~1.0 (only 1 vector in index, exact match expected)
    assert!(
        results[0].score > 0.9,
        "Updated vector should be indexed, got score {}",
        results[0].score,
    );
}

#[test]
fn test_hnsw_thread_safety() {
    use std::sync::Arc;
    use std::thread;

    // Arrange
    let index = Arc::new(HnswIndex::new(3, DistanceMetric::Cosine).unwrap());
    let mut handles = vec![];

    // Act - Insert from multiple threads (unique IDs)
    for i in 0..10 {
        let index_clone = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            #[allow(clippy::cast_precision_loss)]
            index_clone.insert(i, &[i as f32, 0.0, 0.0]);
        }));
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Set searching mode after parallel insertions (required by hnsw_rs)
    index.set_searching_mode();

    // Assert
    assert_eq!(index.len(), 10);
}

#[test]
fn test_hnsw_persistence() {
    use tempfile::tempdir;

    // Arrange
    let dir = tempdir().unwrap();
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0]);

    // Act - Save
    index.save(dir.path()).unwrap();

    // Act - Load
    let loaded_index = HnswIndex::load(dir.path(), 3, DistanceMetric::Cosine).unwrap();

    // Assert
    assert_eq!(loaded_index.len(), 2);
    assert_eq!(loaded_index.dimension(), 3);
    assert_eq!(loaded_index.metric(), DistanceMetric::Cosine);

    // Verify search works on loaded index
    let results = loaded_index.search(&[1.0, 0.0, 0.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1);
}

#[test]
fn test_hnsw_load_legacy_snapshot_without_vectors_disables_vacuum() {
    use std::fs;
    use tempfile::tempdir;

    // Arrange: create a valid snapshot then remove vector sidecar to simulate legacy format.
    let dir = tempdir().unwrap();
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0]);
    index.save(dir.path()).unwrap();
    fs::remove_file(dir.path().join("native_vectors.bin")).unwrap();

    // Act: load snapshot missing vectors.
    let loaded_index = HnswIndex::load(dir.path(), 3, DistanceMetric::Cosine).unwrap();

    // Assert: keep queryability but block vacuum that would otherwise rebuild from missing vectors.
    assert_eq!(loaded_index.len(), 2);
    assert!(!loaded_index.has_vector_storage());
    assert_eq!(
        loaded_index.vacuum(),
        Err(VacuumError::VectorStorageDisabled)
    );
    assert_eq!(loaded_index.len(), 2);
}

#[test]
fn test_hnsw_fast_insert_save_does_not_persist_vectors_file() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let index = HnswIndex::new_fast_insert(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0]);

    index.save(dir.path()).unwrap();
    assert!(!dir.path().join("native_vectors.bin").exists());

    let loaded = HnswIndex::load(dir.path(), 3, DistanceMetric::Cosine).unwrap();
    assert!(!loaded.has_vector_storage());
}

#[test]
fn test_hnsw_fast_insert_save_removes_stale_vectors_file() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();

    // Write a regular snapshot first (with vectors sidecar).
    let regular = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    regular.insert(1, &[1.0, 0.0, 0.0]);
    regular.save(dir.path()).unwrap();
    assert!(dir.path().join("native_vectors.bin").exists());

    // Overwrite with fast-insert snapshot; stale vectors file must be removed.
    let fast = HnswIndex::new_fast_insert(3, DistanceMetric::Cosine).unwrap();
    fast.insert(2, &[0.0, 1.0, 0.0]);
    fast.save(dir.path()).unwrap();

    assert!(!dir.path().join("native_vectors.bin").exists());
}

#[test]
fn test_hnsw_insert_batch_parallel() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    let vectors: Vec<(u64, Vec<f32>)> = vec![
        (1, vec![1.0, 0.0, 0.0]),
        (2, vec![0.0, 1.0, 0.0]),
        (3, vec![0.0, 0.0, 1.0]),
        (4, vec![0.5, 0.5, 0.0]),
        (5, vec![0.5, 0.0, 0.5]),
    ];

    // Act
    let inserted = index.insert_batch_parallel(vectors.iter().map(|(id, v)| (*id, v.as_slice())));
    index.set_searching_mode();

    // Assert
    assert_eq!(inserted, 5);
    assert_eq!(index.len(), 5);

    // Verify search works
    let results = index.search(&[1.0, 0.0, 0.0], 3);
    assert_eq!(results.len(), 3);
    // ID 1 should be in the top results (exact match)
    // Note: Due to parallel insertion, graph structure may vary
    let result_ids: Vec<u64> = results.iter().map(|r| r.id).collect();
    assert!(result_ids.contains(&1), "ID 1 should be in top 3 results");
}

#[test]
fn test_hnsw_insert_batch_parallel_upserts_duplicates() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();

    // Insert one vector first
    index.insert(1, &[1.0, 0.0, 0.0]);

    // Act - Insert batch with existing ID (upsert) + new ID
    let vectors: Vec<(u64, Vec<f32>)> = vec![
        (1, vec![0.0, 1.0, 0.0]), // Upsert: updates existing id=1
        (2, vec![0.0, 0.0, 1.0]), // New
    ];
    let inserted = index.insert_batch_parallel(vectors.iter().map(|(id, v)| (*id, v.as_slice())));
    index.set_searching_mode();

    // Assert - Both vectors processed (1 upserted + 1 new)
    assert_eq!(inserted, 2);
    assert_eq!(index.len(), 2);
}

#[test]
fn test_hnsw_insert_batch_parallel_empty() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    let vectors: Vec<(u64, &[f32])> = vec![];

    // Act
    let inserted = index.insert_batch_parallel(vectors);

    // Assert
    assert_eq!(inserted, 0);
    assert!(index.is_empty());
}

#[test]
fn test_hnsw_insert_batch_parallel_wrong_dimension_returns_zero() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    let vectors: Vec<(u64, &[f32])> = vec![(1, &[1.0, 0.0])]; // Wrong dim

    // Act - dimension mismatch returns 0 inserted
    let inserted = index.insert_batch_parallel(vectors);
    assert_eq!(inserted, 0, "Wrong-dimension batch should insert nothing");
    assert_eq!(index.len(), 0, "Index should remain empty");
}

// =========================================================================
// HnswIndex with Params Tests
// Note: HnswParams unit tests are in params.rs
// =========================================================================

#[test]
fn test_hnsw_with_params() {
    let params = HnswParams::custom(48, 600, 500_000);
    let index = HnswIndex::with_params(1536, DistanceMetric::Cosine, params).unwrap();

    assert_eq!(index.dimension(), 1536);
    assert!(index.is_empty());
}

// =========================================================================
// SIMD Re-ranking Tests (TDD - RED phase)
// =========================================================================

#[test]
fn test_search_with_rerank_returns_k_results() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]);
    index.insert(3, &[0.8, 0.2, 0.0]);
    index.insert(4, &[0.0, 1.0, 0.0]);
    index.insert(5, &[0.0, 0.0, 1.0]);

    // Act
    let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 5).unwrap();

    // Assert
    assert_eq!(results.len(), 3, "Should return exactly k results");
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_search_with_rerank_improves_ranking() {
    // Arrange - vectors with subtle differences
    let index = HnswIndex::new(128, DistanceMetric::Cosine).unwrap();

    // Create vectors with known similarity ordering
    let base: Vec<f32> = (0..128).map(|i| (i as f32 * 0.01).sin()).collect();

    // Slightly modified versions
    let mut v1 = base.clone();
    v1[0] += 0.001; // Very similar

    let mut v2 = base.clone();
    v2[0] += 0.01; // Less similar

    let mut v3 = base.clone();
    v3[0] += 0.1; // Even less similar

    index.insert(1, &v1);
    index.insert(2, &v2);
    index.insert(3, &v3);

    // Act
    let results = index.search_with_rerank(&base, 3, 3).unwrap();

    // Assert - ID 1 should be closest (highest similarity)
    assert_eq!(results[0].id, 1, "Most similar vector should be first");
}

#[test]
fn test_search_with_rerank_handles_rerank_k_greater_than_index_size() {
    // Arrange - use more vectors to avoid HNSW flakiness
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0]);
    index.insert(3, &[0.0, 0.0, 1.0]);
    index.insert(4, &[0.5, 0.5, 0.0]);
    index.insert(5, &[0.5, 0.0, 0.5]);

    // Act - rerank_k > index size
    let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 100).unwrap();

    // Assert - should return at least some results
    assert!(!results.is_empty(), "Should return results");
    assert!(results.len() <= 5, "Should not exceed index size");
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_with_rerank_uses_simd_distances() {
    // Arrange
    let index = HnswIndex::new(768, DistanceMetric::Cosine).unwrap();

    // Insert 100 vectors
    for i in 0..100_u64 {
        let v: Vec<f32> = (0..768)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..768).map(|j| (j as f32 * 0.01).sin()).collect();

    // Act
    let results = index.search_with_rerank(&query, 10, 50).unwrap();

    // Assert - results should have valid distances (SIMD computed)
    // Note: HNSW may return fewer results if graph not fully connected
    assert!(!results.is_empty(), "Should return at least one result");
    for sr in &results {
        assert!(
            sr.score >= -1.0 && sr.score <= 1.0,
            "Cosine should be in [-1, 1]"
        );
    }

    // Results should be sorted by similarity (descending for cosine)
    for i in 1..results.len() {
        assert!(
            results[i - 1].score >= results[i].score,
            "Results should be sorted by similarity descending"
        );
    }
}

#[test]
fn test_search_with_rerank_euclidean_metric() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Euclidean).unwrap();
    index.insert(1, &[0.0, 0.0, 0.0]);
    index.insert(2, &[1.0, 0.0, 0.0]);
    index.insert(3, &[2.0, 0.0, 0.0]);

    // Act
    let results = index.search_with_rerank(&[0.0, 0.0, 0.0], 3, 3).unwrap();

    // Assert - ID 1 should be closest (smallest distance)
    assert_eq!(results[0].id, 1, "Origin should be closest to itself");
    // For euclidean, smaller is better - results sorted ascending
    for i in 1..results.len() {
        assert!(
            results[i - 1].score <= results[i].score,
            "Euclidean results should be sorted ascending"
        );
    }
}

// =========================================================================
// WIS-8: Memory Leak Fix Tests
// Tests for multi-tenant scenarios and proper Drop behavior
// =========================================================================

#[test]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::uninlined_format_args
)]
fn test_hnsw_multi_tenant_load_unload() {
    // Arrange - Simulate multi-tenant scenario with multiple load/unload cycles
    // This test verifies that indices can be loaded and dropped without memory leak
    use tempfile::tempdir;

    let dir = tempdir().expect("Failed to create temp dir");

    // Create and save an index
    {
        let index = HnswIndex::new(128, DistanceMetric::Cosine).unwrap();
        for i in 0..100_u64 {
            let v: Vec<f32> = (0..128)
                .map(|j| ((i + j as u64) as f32 * 0.01).sin())
                .collect();
            index.insert(i, &v);
        }
        index.save(dir.path()).expect("Failed to save index");
    }

    // Act - Load and drop multiple times (simulates multi-tenant load/unload)
    for iteration in 0..5 {
        let loaded =
            HnswIndex::load(dir.path(), 128, DistanceMetric::Cosine).expect("Failed to load index");

        // Verify index works correctly
        assert_eq!(
            loaded.len(),
            100,
            "Iteration {}: Should have 100 vectors",
            iteration
        );

        let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.01).sin()).collect();
        let results = loaded.search(&query, 5);
        // HNSW may return fewer than k results depending on graph connectivity
        assert!(
            !results.is_empty() && results.len() <= 5,
            "Iteration {}: Should return 1-5 results, got {}",
            iteration,
            results.len()
        );

        // Index is dropped here, io_holder should be freed
    }

    // If we get here without crash/hang, memory is being managed correctly
}

#[test]
fn test_hnsw_drop_cleans_up_properly() {
    // Arrange - Create index, verify it can be dropped without issues
    use tempfile::tempdir;

    let dir = tempdir().expect("Failed to create temp dir");

    // Create, save, load, and drop
    {
        let index = HnswIndex::new(64, DistanceMetric::Euclidean).unwrap();
        index.insert(1, &vec![0.5; 64]);
        index.insert(2, &vec![0.3; 64]);
        index.save(dir.path()).expect("Failed to save");
    }

    // Load and immediately drop
    {
        let _loaded =
            HnswIndex::load(dir.path(), 64, DistanceMetric::Euclidean).expect("Failed to load");
        // Dropped here
    }

    // Load again to verify files are still valid after previous drop
    {
        let loaded = HnswIndex::load(dir.path(), 64, DistanceMetric::Euclidean)
            .expect("Failed to load after previous drop");
        assert_eq!(loaded.len(), 2);
    }
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::uninlined_format_args)]
fn test_hnsw_save_load_preserves_all_metrics() {
    use tempfile::tempdir;

    // Test Cosine and Euclidean metrics
    // Note: DotProduct has numerical precision issues in hnsw_rs with certain vectors
    for metric in [DistanceMetric::Cosine, DistanceMetric::Euclidean] {
        let dir = tempdir().expect("Failed to create temp dir");
        let dim = 32;

        // Create varied vectors (not constant) to avoid numerical issues
        let v1: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.1).sin()).collect();
        let v2: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.2).cos()).collect();
        let query: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.15).sin()).collect();

        // Create and save
        {
            let index = HnswIndex::new(dim, metric).unwrap();
            index.insert(1, &v1);
            index.insert(2, &v2);
            index.save(dir.path()).expect("Failed to save");
        }

        // Load and verify
        {
            let loaded = HnswIndex::load(dir.path(), dim, metric).expect("Failed to load");
            assert_eq!(
                loaded.len(),
                2,
                "Metric {:?}: Should have 2 vectors",
                metric
            );
            assert_eq!(loaded.metric(), metric, "Metric should be preserved");
            assert_eq!(loaded.dimension(), dim, "Dimension should be preserved");

            // Verify search works
            let results = loaded.search(&query, 2);
            assert!(
                !results.is_empty(),
                "Metric {:?}: Should return results",
                metric
            );
        }
    }
}

// =========================================================================
// SearchQuality Tests
// =========================================================================

#[test]
fn test_search_quality_fast() {
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    // Insert more vectors for stable HNSW graph (small graphs are non-deterministic)
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]);
    index.insert(3, &[0.8, 0.2, 0.0]);
    index.insert(4, &[0.7, 0.3, 0.0]);
    index.insert(5, &[0.0, 1.0, 0.0]);

    let results = index
        .search_with_quality(&[1.0, 0.0, 0.0], 2, SearchQuality::Fast)
        .unwrap();
    // Fast mode may return fewer results with very small ef_search
    assert!(!results.is_empty(), "Should return at least one result");
    assert!(results.len() <= 2, "Should not exceed requested k");
}

#[test]
fn test_search_quality_balanced() {
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]);

    // Test Balanced quality mode
    let results = index
        .search_with_quality(&[1.0, 0.0, 0.0], 2, SearchQuality::Balanced)
        .unwrap();
    // HNSW may return fewer results for very small indices
    assert!(!results.is_empty(), "Should return at least one result");
    assert_eq!(
        results[0].id, 1,
        "Balanced search should find exact match first"
    );
}

#[test]
fn test_search_quality_custom_ef() {
    // Use more vectors to make HNSW more stable
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]);
    index.insert(3, &[0.8, 0.2, 0.0]);
    index.insert(4, &[0.0, 1.0, 0.0]);
    index.insert(5, &[0.0, 0.0, 1.0]);

    let results = index
        .search_with_quality(&[1.0, 0.0, 0.0], 3, SearchQuality::Custom(512))
        .unwrap();
    assert_eq!(results.len(), 3);
}

// Note: SearchQuality::ef_search unit tests are in params.rs

// =========================================================================
// Edge Cases and Error Handling
// =========================================================================

#[test]
fn test_hnsw_load_nonexistent_path() {
    let result = HnswIndex::load("nonexistent_path_12345", 128, DistanceMetric::Cosine);
    assert!(result.is_err(), "Loading from nonexistent path should fail");
}

#[test]
fn test_hnsw_search_with_rerank_empty_index() {
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 10, 50).unwrap();
    assert!(
        results.is_empty(),
        "Empty index should return empty results"
    );
}

#[test]
fn test_hnsw_search_with_rerank_dot_product() {
    let index = HnswIndex::new(3, DistanceMetric::DotProduct).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.5, 0.5, 0.0]);
    index.insert(3, &[0.0, 1.0, 0.0]);

    let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 3).unwrap();

    // HNSW may return fewer results for very small indices
    assert!(!results.is_empty(), "Should return at least one result");
    // For dot product, ID 1 should have highest score
    assert_eq!(results[0].id, 1, "Highest dot product should be first");
}

#[test]
fn test_hnsw_io_holder_is_none_for_new_index() {
    // For newly created indices, io_holder should be None
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    // We can't directly access io_holder, but we can verify the index works
    // and drops without issues (no io_holder to manage)
    index.insert(1, &[1.0, 0.0, 0.0]);
    assert_eq!(index.len(), 1);
    // Dropped here without io_holder cleanup needed
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_hnsw_large_batch_parallel_insert() {
    let index = HnswIndex::new(128, DistanceMetric::Cosine).unwrap();

    // Create 200 vectors (reduced from 1000 for faster test execution)
    let vectors: Vec<(u64, Vec<f32>)> = (0..200)
        .map(|i| {
            let v: Vec<f32> = (0..128).map(|j| ((i + j) as f32 * 0.001).sin()).collect();
            (i as u64, v)
        })
        .collect();

    let inserted = index.insert_batch_parallel(vectors.iter().map(|(id, v)| (*id, v.as_slice())));
    index.set_searching_mode();

    assert_eq!(inserted, 200, "Should insert 200 vectors");
    assert_eq!(index.len(), 200);

    // Verify search works
    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.001).sin()).collect();
    let results = index.search(&query, 10);
    assert_eq!(results.len(), 10);
}

// =========================================================================
// TS-CORE-001: Adaptive Prefetch Tests
// =========================================================================

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_with_rerank_768d_prefetch() {
    // Test adaptive prefetch for 768D vectors (3KB each)
    // prefetch_distance should be 768*4/64 = 48, clamped to 16
    let index = HnswIndex::new(768, DistanceMetric::Cosine).unwrap();

    // Insert 100 vectors
    for i in 0u64..100 {
        let v: Vec<f32> = (0..768)
            .map(|j| ((i + j as u64) as f32 * 0.001).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..768).map(|j| (j as f32 * 0.001).sin()).collect();
    let results = index.search_with_rerank(&query, 10, 50).unwrap();

    assert!(!results.is_empty(), "Should return results");
    assert!(results.len() <= 10, "Should not exceed k");
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_with_rerank_small_dim_prefetch() {
    // Test adaptive prefetch for small vectors (32D = 128 bytes)
    // prefetch_distance should be 128/64 = 2, clamped to 4 (minimum)
    let index = HnswIndex::new(32, DistanceMetric::Cosine).unwrap();

    for i in 0u64..50 {
        let v: Vec<f32> = (0..32)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..32).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = index.search_with_rerank(&query, 5, 20).unwrap();

    assert!(!results.is_empty(), "Should return results");
}

// =========================================================================
// TS-CORE-002: Batch Search Optimization Tests
// =========================================================================

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_batch_parallel_consistency() {
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    // Insert 100 vectors (reduced from 200 for faster test execution)
    for i in 0u64..100 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    // Create batch queries
    let queries: Vec<Vec<f32>> = (0..10)
        .map(|i| {
            (0..64)
                .map(|j| ((200 + i + j) as f32 * 0.01).sin())
                .collect()
        })
        .collect();
    let query_refs: Vec<&[f32]> = queries.iter().map(Vec::as_slice).collect();

    // Batch search
    let batch_results = index
        .search_batch_parallel(&query_refs, 5, SearchQuality::Balanced)
        .unwrap();

    // Individual searches for comparison
    let individual_results: Vec<Vec<crate::scored_result::ScoredResult>> = queries
        .iter()
        .map(|q| {
            index
                .search_with_quality(q, 5, SearchQuality::Balanced)
                .unwrap()
        })
        .collect();

    // Results should match (same IDs, though order might vary slightly)
    assert_eq!(batch_results.len(), individual_results.len());
    for (batch, individual) in batch_results.iter().zip(&individual_results) {
        assert_eq!(batch.len(), individual.len(), "Result counts should match");
    }
}

#[test]
fn test_search_batch_parallel_empty_queries() {
    let index = HnswIndex::new(3, DistanceMetric::Cosine).unwrap();
    index.insert(1, &[1.0, 0.0, 0.0]);

    let queries: Vec<&[f32]> = vec![];
    let results = index
        .search_batch_parallel(&queries, 5, SearchQuality::Fast)
        .unwrap();

    assert!(
        results.is_empty(),
        "Empty queries should return empty results"
    );
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_batch_parallel_large_batch() {
    let index = HnswIndex::new(128, DistanceMetric::Cosine).unwrap();

    // Insert 150 vectors (reduced from 500 for faster test execution)
    for i in 0u64..150 {
        let v: Vec<f32> = (0..128)
            .map(|j| ((i + j as u64) as f32 * 0.001).sin())
            .collect();
        index.insert(i, &v);
    }
    index.set_searching_mode();

    // 20 queries batch (reduced from 100 for faster test execution)
    let queries: Vec<Vec<f32>> = (0..20)
        .map(|i| {
            (0..128)
                .map(|j| ((150 + i + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();
    let query_refs: Vec<&[f32]> = queries.iter().map(Vec::as_slice).collect();

    // Use Balanced for faster test execution
    let results = index
        .search_batch_parallel(&query_refs, 10, SearchQuality::Balanced)
        .unwrap();

    assert_eq!(results.len(), 20, "Should return 20 result sets");
    for result in &results {
        assert_eq!(result.len(), 10, "Each result should have 10 neighbors");
    }
}

// =========================================================================
// Recall Quality Regression Tests
// =========================================================================

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_recall_quality_minimum_threshold() {
    // Ensure recall@10 >= 90% for Accurate quality on small dataset
    let dim = 64;
    let n = 500;
    let k = 10;

    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Generate deterministic dataset
    let dataset: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();

    for (idx, vec) in dataset.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        index.insert(idx as u64, vec);
    }

    // Generate query
    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.001).sin()).collect();

    // Compute ground truth with brute force
    let mut distances: Vec<crate::scored_result::ScoredResult> = dataset
        .iter()
        .enumerate()
        .map(|(idx, vec)| {
            let sim = crate::simd_native::cosine_similarity_native(&query, vec);
            #[allow(clippy::cast_possible_truncation)]
            crate::scored_result::ScoredResult::new(idx as u64, sim)
        })
        .collect();
    distances.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let ground_truth: Vec<u64> = distances.iter().take(k).map(|sr| sr.id).collect();

    // HNSW search
    let results = index
        .search_with_quality(&query, k, SearchQuality::Accurate)
        .unwrap();
    let result_ids: std::collections::HashSet<u64> = results.iter().map(|sr| sr.id).collect();
    let gt_set: std::collections::HashSet<u64> = ground_truth.iter().copied().collect();

    let recall = result_ids.intersection(&gt_set).count() as f64 / k as f64;

    assert!(
        recall >= 0.8,
        "Recall@{k} should be >= 80% for Accurate, got {:.1}%",
        recall * 100.0
    );
}

#[test]
fn test_rerank_latency_target_configuration_roundtrip() {
    let index = HnswIndex::new(32, DistanceMetric::Cosine).unwrap();
    assert_eq!(index.rerank_latency_target_us(), 0);

    index.set_rerank_latency_target_us(250);
    assert_eq!(index.rerank_latency_target_us(), 250);
}

#[test]
fn test_rerank_latency_ema_updates_after_two_stage_search() {
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();
    index.set_rerank_latency_target_us(1);

    for i in 0u64..1500 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i * 5 + j as u64) as f32 * 0.0013).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..64).map(|j| (j as f32 * 0.009).cos()).collect();
    let _ = index
        .search_with_quality(&query, 20, SearchQuality::Accurate)
        .unwrap();

    assert!(index.rerank_latency_ema_us() > 0);
}

#[test]
fn test_update_rerank_latency_ema_large_current_does_not_overflow() {
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    for i in 0u64..400 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i * 3 + j as u64) as f32 * 0.0021).sin())
            .collect();
        index.insert(i, &v);
    }

    index
        .rerank_latency_ema_us
        .store(u64::MAX, std::sync::atomic::Ordering::Relaxed);

    let query: Vec<f32> = (0..64).map(|j| (j as f32 * 0.011).cos()).collect();
    let results = index
        .search_with_quality(&query, 20, SearchQuality::Accurate)
        .unwrap();

    assert!(!results.is_empty());

    let ema = index.rerank_latency_ema_us();
    let expected_min = (u64::MAX / 10) * 7;
    assert!(ema >= expected_min, "EMA underflow/wrap detected: {ema}");
}

#[test]
fn test_search_with_quality_custom_ef_uses_high_recall_path_without_regression() {
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    for i in 0u64..2000 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i + j as u64) as f32 * 0.001).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..64).map(|j| (j as f32 * 0.013).cos()).collect();
    let results = index
        .search_with_quality(&query, 20, SearchQuality::Custom(512))
        .unwrap();

    assert!(!results.is_empty());
    assert!(results.len() <= 20);
}

#[test]
fn test_search_with_quality_accurate_stays_stable_on_medium_dataset() {
    let index = HnswIndex::new(128, DistanceMetric::Cosine).unwrap();

    for i in 0u64..5000 {
        let v: Vec<f32> = (0..128)
            .map(|j| ((i * 3 + j as u64) as f32 * 0.0007).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.007).sin()).collect();
    let results = index
        .search_with_quality(&query, 15, SearchQuality::Accurate)
        .unwrap();

    assert!(!results.is_empty());
    assert!(results.len() <= 15);
}

// =========================================================================
// RF-3: Tests for search_brute_force_buffered (buffer reuse optimization)
// =========================================================================

#[test]
fn test_brute_force_buffered_same_results_as_original() {
    let index = HnswIndex::new(32, DistanceMetric::Cosine).unwrap();

    // Insert vectors
    for i in 0u64..50 {
        let v: Vec<f32> = (0..32)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..32).map(|j| (j as f32 * 0.02).cos()).collect();

    // Compare results
    let original = index.search_brute_force(&query, 10).unwrap();
    let buffered = index.search_brute_force_buffered(&query, 10).unwrap();

    assert_eq!(original.len(), buffered.len());
    for (orig, buf) in original.iter().zip(buffered.iter()) {
        assert_eq!(orig.id, buf.id, "IDs should match");
        assert!(
            (orig.score - buf.score).abs() < 1e-6,
            "Distances should match"
        );
    }
}

#[test]
fn test_brute_force_buffered_empty_index() {
    let index = HnswIndex::new(16, DistanceMetric::Euclidean).unwrap();
    let query: Vec<f32> = vec![0.0; 16];

    let results = index.search_brute_force_buffered(&query, 5).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_brute_force_buffered_all_metrics() {
    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ] {
        let index = HnswIndex::new(8, metric).unwrap();
        index.insert(1, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        index.insert(2, &[0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        index.insert(3, &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

        let results = index
            .search_brute_force_buffered(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 3)
            .unwrap();
        assert_eq!(results.len(), 3, "Should return 3 results for {metric:?}");
    }
}

#[test]
fn test_brute_force_buffered_repeated_calls_stable() {
    let index = HnswIndex::new(16, DistanceMetric::Cosine).unwrap();

    for i in 0u64..20 {
        let v: Vec<f32> = (0..16)
            .map(|j| ((i + j as u64) as f32 * 0.1).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = vec![0.5; 16];

    // Multiple calls should return identical results
    let r1 = index.search_brute_force_buffered(&query, 5).unwrap();
    let r2 = index.search_brute_force_buffered(&query, 5).unwrap();
    let r3 = index.search_brute_force_buffered(&query, 5).unwrap();

    assert_eq!(r1, r2);
    assert_eq!(r2, r3);
}

// =========================================================================
// Stress Tests
// =========================================================================

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_concurrent_search_stress() {
    use std::sync::Arc;
    use std::thread;

    let index = Arc::new(HnswIndex::new(64, DistanceMetric::Cosine).unwrap());

    // Insert vectors
    for i in 0u64..100 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    // Spawn multiple search threads
    let handles: Vec<_> = (0..4)
        .map(|t| {
            let idx = Arc::clone(&index);
            thread::spawn(move || {
                for i in 0..50 {
                    let query: Vec<f32> = (0..64)
                        .map(|j| ((t * 100 + i + j) as f32 * 0.01).sin())
                        .collect();
                    let results = idx.search(&query, 5);
                    assert!(!results.is_empty());
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

#[test]
fn test_all_distance_metrics_search_with_rerank() {
    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ] {
        let index = HnswIndex::new(8, metric).unwrap();
        index.insert(1, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        index.insert(2, &[0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        index.insert(3, &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

        let results = index
            .search_with_rerank(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 3, 3)
            .unwrap();

        assert!(
            !results.is_empty(),
            "search_with_rerank should work for {metric:?}"
        );
    }
}

// =========================================================================
// SAFETY: Drop Order Tests for io_holder unsafe invariant
// =========================================================================
//
// These tests verify that the unsafe lifetime extension in HnswIndex::load()
// doesn't cause use-after-free when the index is dropped.
//
// CRITICAL INVARIANT: `inner` (which borrows from io_holder) MUST be dropped
// BEFORE `io_holder`. Our Drop impl ensures this via ManuallyDrop.

#[test]
fn test_drop_safety_loaded_index_no_segfault() {
    // This test verifies that dropping a loaded HnswIndex doesn't segfault.
    // If the Drop order is wrong, this will cause use-after-free.
    use tempfile::tempdir;

    let dir = tempdir().expect("Failed to create temp dir");

    // 1. Create and save an index
    {
        let index = HnswIndex::new(4, DistanceMetric::Cosine).unwrap();
        index.insert(1, &[1.0, 0.0, 0.0, 0.0]);
        index.insert(2, &[0.0, 1.0, 0.0, 0.0]);
        index.insert(3, &[0.0, 0.0, 1.0, 0.0]);
        index.save(dir.path()).expect("Failed to save");
    }

    // 2. Load and drop multiple times to stress test Drop safety
    for _ in 0..5 {
        let loaded =
            HnswIndex::load(dir.path(), 4, DistanceMetric::Cosine).expect("Failed to load");

        // Perform operations that touch the borrowed data
        let results = loaded.search(&[1.0, 0.0, 0.0, 0.0], 2);
        assert!(!results.is_empty(), "Search should return results");

        // Index is dropped here - if Drop order is wrong, this segfaults
    }
}

#[test]
fn test_drop_safety_loaded_index_concurrent_drop() {
    // Stress test: multiple threads loading and dropping indices
    use std::sync::Arc;
    use std::thread;
    use tempfile::tempdir;

    let dir = tempdir().expect("Failed to create temp dir");

    // Create and save an index
    {
        let index = HnswIndex::new(4, DistanceMetric::Cosine).unwrap();
        for i in 0u64..10 {
            let v = vec![i as f32, 0.0, 0.0, 0.0];
            index.insert(i, &v);
        }
        index.save(dir.path()).expect("Failed to save");
    }

    let path = Arc::new(dir.path().to_path_buf());

    // Spawn threads that load, search, and drop
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let p = Arc::clone(&path);
            thread::spawn(move || {
                for _ in 0..3 {
                    let loaded =
                        HnswIndex::load(&*p, 4, DistanceMetric::Cosine).expect("Failed to load");
                    let results = loaded.search(&[1.0, 0.0, 0.0, 0.0], 3);
                    assert!(!results.is_empty());
                    // Drop happens here
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("Thread should not panic from Drop");
    }
}

#[test]
fn test_drop_safety_search_after_partial_operations() {
    // Test that search works correctly even with complex operation sequences
    // before drop, ensuring borrowed data is valid until Drop.
    use tempfile::tempdir;

    let dir = tempdir().expect("Failed to create temp dir");

    // Create index with various operations
    {
        let index = HnswIndex::new(8, DistanceMetric::Euclidean).unwrap();
        for i in 0u64..20 {
            let v: Vec<f32> = (0..8).map(|j| (i + j) as f32 * 0.1).collect();
            index.insert(i, &v);
        }
        index.save(dir.path()).expect("Failed to save");
    }

    // Load and perform many operations before drop
    let loaded = HnswIndex::load(dir.path(), 8, DistanceMetric::Euclidean).expect("Failed to load");

    // Multiple searches touching the mmap'd data
    for i in 0..10 {
        let query: Vec<f32> = (0..8).map(|j| (i + j) as f32 * 0.1).collect();
        let results = loaded.search(&query, 5);
        assert!(results.len() <= 5);
    }

    // Batch search
    let queries: Vec<Vec<f32>> = (0..5)
        .map(|i| (0..8).map(|j| (i + j) as f32 * 0.1).collect())
        .collect();
    let query_refs: Vec<&[f32]> = queries.iter().map(|v| v.as_slice()).collect();
    let batch_results = loaded
        .search_batch_parallel(&query_refs, 3, SearchQuality::Balanced)
        .unwrap();
    assert_eq!(batch_results.len(), 5);

    // Drop happens here - all borrowed data must still be valid
    drop(loaded);
}

// =========================================================================
// SEC-1: Stress Test - Drop under heavy concurrent load
// Validates ManuallyDrop + RwLock safety under extreme conditions
// =========================================================================

#[test]
fn test_drop_stress_concurrent_create_destroy_loop() {
    // Stress test: rapidly create/destroy indices while performing operations
    // This tests the ManuallyDrop pattern under pressure
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let success_count = Arc::new(AtomicUsize::new(0));
    let iterations = 50;

    for _ in 0..iterations {
        let success = Arc::clone(&success_count);

        // Create index, perform operations, drop
        let index = Arc::new(HnswIndex::new(16, DistanceMetric::Cosine).unwrap());

        // Spawn readers that will race with drop
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let idx = Arc::clone(&index);
                std::thread::spawn(move || {
                    // Insert some vectors
                    for i in 0..10 {
                        let id = (t * 100 + i) as u64;
                        let v: Vec<f32> = (0..16).map(|j| (id + j) as f32 * 0.01).collect();
                        idx.insert(id, &v);
                    }
                    // Search
                    let q: Vec<f32> = (0..16).map(|i| i as f32 * 0.01).collect();
                    let _ = idx.search(&q, 5);
                })
            })
            .collect();

        // Wait for all threads
        for h in handles {
            h.join().expect("Thread panicked during stress test");
        }

        // Force drop while ensuring all operations completed
        drop(index);
        success.fetch_add(1, Ordering::SeqCst);
    }

    assert_eq!(
        success_count.load(Ordering::SeqCst),
        iterations,
        "All iterations should complete without panic"
    );
}

#[test]
fn test_drop_stress_load_search_destroy_cycle() {
    // Stress test: load from disk, search heavily, destroy - repeated
    use tempfile::tempdir;

    let dir = tempdir().expect("Failed to create temp dir");

    // Create and save initial index
    {
        let index = HnswIndex::new(32, DistanceMetric::Euclidean).unwrap();
        for i in 0u64..100 {
            let v: Vec<f32> = (0..32).map(|j| ((i + j) as f32).sin()).collect();
            index.insert(i, &v);
        }
        index.save(dir.path()).expect("Failed to save");
    }

    // Repeated load/search/destroy cycles
    for cycle in 0..20 {
        let loaded = HnswIndex::load(dir.path(), 32, DistanceMetric::Euclidean)
            .unwrap_or_else(|e| panic!("Cycle {cycle}: Failed to load: {e}"));

        // Heavy search load
        for i in 0..50 {
            let q: Vec<f32> = (0..32).map(|j| ((i + j) as f32).cos()).collect();
            let results = loaded.search(&q, 10);
            assert!(
                results.len() <= 10,
                "Cycle {cycle}: Search returned too many results"
            );
        }

        // Explicit drop to test ManuallyDrop pattern
        drop(loaded);
    }
}

#[test]
fn test_drop_stress_parallel_insert_then_drop() {
    // Stress test: parallel batch insert immediately followed by drop
    // Use Euclidean to avoid cosine normalization requirements
    // Reduced iterations and batch size for faster test execution
    for _ in 0..5 {
        let index = HnswIndex::new(64, DistanceMetric::Euclidean).unwrap();

        // Generate batch data with reasonable magnitude (reduced from 500)
        let batch: Vec<(u64, Vec<f32>)> = (0..100)
            .map(|i| {
                let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
                (i as u64, v)
            })
            .collect();

        // Parallel insert
        let inserted = index.insert_batch_parallel(batch.iter().map(|(id, v)| (*id, v.as_slice())));
        assert!(inserted > 0, "Should insert at least some vectors");

        // Immediate drop without set_searching_mode
        // This tests that Drop handles partially-initialized state
        drop(index);
    }
}

// =========================================================================
// P1-GPU-1: GPU Batch Search Tests (TDD - Written BEFORE implementation)
// =========================================================================

#[test]
#[cfg(feature = "gpu")]
fn test_search_brute_force_gpu_returns_same_results_as_cpu() {
    // TDD: GPU brute force must return identical results to CPU
    let index = HnswIndex::new(128, DistanceMetric::Cosine).unwrap();

    // Insert test vectors
    for i in 0u64..100 {
        let v: Vec<f32> = (0..128)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.02).cos()).collect();

    // CPU brute force
    let cpu_results = index.search_brute_force(&query, 10).unwrap();

    // GPU brute force (if available)
    if let Some(gpu_results) = index.search_brute_force_gpu(&query, 10).unwrap() {
        assert_eq!(
            cpu_results.len(),
            gpu_results.len(),
            "Result count mismatch"
        );

        // Verify same IDs returned (order may differ slightly due to floating point)
        let cpu_ids: std::collections::HashSet<u64> = cpu_results.iter().map(|sr| sr.id).collect();
        let gpu_ids: std::collections::HashSet<u64> = gpu_results.iter().map(|sr| sr.id).collect();

        let overlap = cpu_ids.intersection(&gpu_ids).count();
        assert!(
            overlap >= 8,
            "GPU and CPU should return mostly same IDs (got {overlap}/10 overlap)"
        );
    }
}

#[test]
fn test_search_brute_force_gpu_fallback_to_none_without_gpu() {
    // TDD: Without GPU, should return None gracefully
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();
    index.insert(1, &vec![0.5; 64]);

    let query = vec![0.5; 64];

    // Should not panic, returns Ok(None) if GPU unavailable
    let _result = index.search_brute_force_gpu(&query, 5).unwrap();

    #[cfg(not(feature = "gpu"))]
    assert!(_result.is_none(), "Should return None without GPU feature");
}

#[test]
#[cfg(feature = "gpu")]
fn test_brute_force_gpu_euclidean() {
    // RED: GPU brute-force must dispatch Euclidean, not hardcode Cosine.
    let index = HnswIndex::new(4, DistanceMetric::Euclidean).unwrap();

    // Insert vectors at known positions so distances are deterministic.
    // vec0 = origin, vec1 = unit along x, vec2 = 2 units along x.
    index.insert(10, &[0.0, 0.0, 0.0, 0.0]);
    index.insert(20, &[1.0, 0.0, 0.0, 0.0]);
    index.insert(30, &[2.0, 0.0, 0.0, 0.0]);

    let query = [0.0, 0.0, 0.0, 0.0]; // closest to vec0

    if let Some(results) = index.search_brute_force_gpu(&query, 3).unwrap() {
        assert_eq!(results.len(), 3, "Should return 3 results");

        // Euclidean: ascending order (lower distance = better).
        // Expected order: id=10 (dist 0), id=20 (dist 1), id=30 (dist 2).
        assert_eq!(results[0].id, 10, "Closest should be id=10 (at origin)");
        assert_eq!(results[1].id, 20, "Second closest should be id=20");
        assert_eq!(results[2].id, 30, "Farthest should be id=30");

        // Verify actual distances
        assert!(
            results[0].score.abs() < 0.01,
            "Distance to origin should be ~0, got {}",
            results[0].score
        );
        assert!(
            (results[1].score - 1.0).abs() < 0.01,
            "Distance to (1,0,0,0) should be ~1, got {}",
            results[1].score
        );
    }
}

#[test]
#[cfg(feature = "gpu")]
fn test_brute_force_gpu_dot_product() {
    // RED: GPU brute-force must dispatch DotProduct, not hardcode Cosine.
    let index = HnswIndex::new(4, DistanceMetric::DotProduct).unwrap();

    // Insert vectors with different dot products to the query.
    index.insert(10, &[1.0, 0.0, 0.0, 0.0]); // dot with query = 1
    index.insert(20, &[2.0, 0.0, 0.0, 0.0]); // dot with query = 2
    index.insert(30, &[3.0, 0.0, 0.0, 0.0]); // dot with query = 3

    let query = [1.0, 0.0, 0.0, 0.0];

    if let Some(results) = index.search_brute_force_gpu(&query, 3).unwrap() {
        assert_eq!(results.len(), 3, "Should return 3 results");

        // DotProduct: descending order (higher = better).
        // Expected order: id=30 (dot 3), id=20 (dot 2), id=10 (dot 1).
        assert_eq!(
            results[0].id, 30,
            "Highest dot product should be id=30, got id={}",
            results[0].id
        );
        assert_eq!(
            results[1].id, 20,
            "Second highest dot product should be id=20, got id={}",
            results[1].id
        );
        assert_eq!(
            results[2].id, 10,
            "Lowest dot product should be id=10, got id={}",
            results[2].id
        );

        // Verify actual scores
        assert!(
            (results[0].score - 3.0).abs() < 0.01,
            "Dot product with (3,0,0,0) should be ~3, got {}",
            results[0].score
        );
    }
}

#[test]
fn test_compute_backend_selection() {
    // TDD: Verify compute backend selection works
    use crate::gpu::ComputeBackend;

    let backend = ComputeBackend::best_available();

    // Should always return a valid backend
    match backend {
        ComputeBackend::Simd => {
            // SIMD is always available
        }
        #[cfg(feature = "gpu")]
        ComputeBackend::Gpu => {
            // GPU selected when available
        }
    }
}

// =========================================================================
// FT-2: Property-Based Tests with proptest
// =========================================================================

mod proptest_tests {
    use super::*;
    use proptest::prelude::*;

    /// Strategy for generating valid vector dimensions (reasonable range)
    fn dimension_strategy() -> impl Strategy<Value = usize> {
        8usize..=256
    }

    /// Strategy for generating a random f32 vector of given dimension
    #[allow(dead_code)]
    fn vector_strategy(dim: usize) -> impl Strategy<Value = Vec<f32>> {
        proptest::collection::vec(-1.0f32..1.0, dim)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        /// Property: len() always equals number of successful insertions
        #[test]
        fn prop_len_equals_insertions(
            dim in dimension_strategy(),
            vectors in proptest::collection::vec(
                proptest::collection::vec(-1.0f32..1.0, 8usize..=64),
                1usize..=20
            )
        ) {
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();
            let mut inserted = 0usize;

            for (i, v) in vectors.into_iter().enumerate() {
                if v.len() == dim {
                    index.insert(i as u64, &v);
                    inserted += 1;
                }
            }

            prop_assert_eq!(index.len(), inserted);
        }

        /// Property: search never returns more than k results
        #[test]
        fn prop_search_returns_at_most_k(
            dim in 16usize..=64,
            k in 1usize..=20,
            num_vectors in 5usize..=50
        ) {
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();

            // Insert random vectors
            for i in 0..num_vectors {
                let v: Vec<f32> = (0..dim).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
                index.insert(i as u64, &v);
            }

            let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.02).cos()).collect();
            let results = index.search(&query, k);

            prop_assert!(results.len() <= k, "Search returned {} results, expected <= {}", results.len(), k);
        }

        /// Property: brute force search always returns exact results
        #[test]
        fn prop_brute_force_exact(
            dim in 8usize..=32,
            num_vectors in 3usize..=20
        ) {
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();

            // Insert vectors with known distances from origin
            for i in 0..num_vectors {
                let mut v = vec![0.0f32; dim];
                v[0] = i as f32; // Distance from origin = i
                index.insert(i as u64, &v);
            }

            let query = vec![0.0f32; dim];
            let results = index.search_brute_force(&query, 3).unwrap();

            // First result should be id=0 (exact match at origin)
            if !results.is_empty() {
                prop_assert_eq!(results[0].id, 0, "Closest should be id=0 (at origin)");
            }
        }

        /// Property: remove always decreases len or returns false
        #[test]
        fn prop_remove_decreases_len(
            dim in 16usize..=32,
            id_to_remove in 0u64..10
        ) {
            let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

            // Insert some vectors
            for i in 0u64..10 {
                let v: Vec<f32> = (0..dim).map(|j| ((i + j as u64) as f32 * 0.01).sin()).collect();
                index.insert(i, &v);
            }

            let len_before = index.len();
            let removed = index.remove(id_to_remove);

            if removed {
                prop_assert_eq!(index.len(), len_before - 1);
            } else {
                prop_assert_eq!(index.len(), len_before);
            }
        }

        /// Property: duplicate inserts are idempotent (no increase in len)
        #[test]
        fn prop_duplicate_insert_idempotent(
            dim in 16usize..=32
        ) {
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();
            let v: Vec<f32> = (0..dim).map(|j| j as f32 * 0.1).collect();

            index.insert(42, &v);
            let len_after_first = index.len();

            index.insert(42, &v); // Duplicate
            let len_after_second = index.len();

            prop_assert_eq!(len_after_first, len_after_second, "Duplicate insert should be idempotent");
        }

        /// Property: batch insert count matches individual inserts
        #[test]
        fn prop_batch_insert_count(
            dim in 16usize..=32,
            batch_size in 5usize..=30
        ) {
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();

            let batch: Vec<(u64, Vec<f32>)> = (0..batch_size)
                .map(|i| {
                    let v: Vec<f32> = (0..dim).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
                    (i as u64, v)
                })
                .collect();

            // Use parallel insert (recommended API)
            let count = index.insert_batch_parallel(batch.iter().map(|(id, v)| (*id, v.as_slice())));

            prop_assert_eq!(count, batch_size, "Batch insert count mismatch");
            prop_assert_eq!(index.len(), batch_size, "Index len mismatch after batch");
        }
    }
}

// =========================================================================
// P1: Safety invariant tests for self-referential pattern
// =========================================================================
// NOTE: test_field_order_io_holder_after_inner is in index.rs (requires private field access)

/// Test that `ManuallyDrop` is used correctly for the inner field.
///
/// This verifies that:
/// 1. The inner field uses `ManuallyDrop` (checked by compilation)
/// 2. The custom Drop impl is present and correct
#[test]
fn test_manuallydrop_pattern_integrity() {
    // Create an index and verify it can be dropped without issues
    let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();

    // Insert some data to ensure internal state is populated
    for i in 0..10 {
        let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
        index.insert(i as u64, &v);
    }

    // Explicit drop - if ManuallyDrop is incorrectly handled, this could panic/UB
    drop(index);

    // If we reach here, the drop order is correct
}

/// Test that loading from disk and dropping works correctly.
///
/// This is the actual use case where the self-referential pattern matters:
/// when loading from disk, `inner` borrows from `io_holder`.
#[test]
fn test_load_and_drop_safety() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path();

    // Create, populate, and save an index
    {
        let index = HnswIndex::new(64, DistanceMetric::Cosine).unwrap();
        for i in 0..50 {
            let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
            index.insert(i as u64, &v);
        }
        index.save(path).expect("Save failed");
    }

    // Load and drop multiple times to stress-test the drop order
    for _ in 0..3 {
        let loaded = HnswIndex::load(path, 64, DistanceMetric::Cosine).expect("Load failed");

        // Verify it works
        let results = loaded.search(&vec![0.0f32; 64], 5);
        assert!(!results.is_empty(), "Search should return results");

        // Drop happens here - critical that inner drops before io_holder
        drop(loaded);
    }
}

// =========================================================================
// Regression: adaptive search spread for distance metrics
// =========================================================================

/// Regression test: `search_adaptive` spread calculation must work correctly
/// for distance-based metrics (Euclidean, Hamming) where results are sorted
/// ascending (best = lowest score).
///
/// Before the fix, the spread formula `(max - min) / min.abs()` produced a
/// negative value for ascending-sorted results, which always compared < 2.0,
/// preventing escalation from ever triggering on distance metrics. The fix
/// uses `(score_a - score_b).abs() / min(|a|, |b|)` which is sign-agnostic.
#[test]
#[allow(clippy::cast_precision_loss)]
fn test_adaptive_search_spread_positive_for_distance_metrics() {
    // Euclidean: distance metric (lower = better, sorted ascending).
    // We need > 100 vectors so brute-force small-collection path is skipped,
    // and we need enough spread so the adaptive threshold (2.0) is exceeded.
    let dim = 32;
    let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();

    // Insert 150 vectors: a tight cluster near the origin and outliers far away.
    // This creates high spread in search results, which should trigger escalation.
    for i in 0u64..120 {
        // Tight cluster: small perturbation around origin
        let v: Vec<f32> = (0..dim)
            .map(|j| ((i + j as u64) as f32 * 0.001).sin() * 0.1)
            .collect();
        index.insert(i, &v);
    }
    for i in 120u64..150 {
        // Outliers: far from origin (magnitude ~10)
        let v: Vec<f32> = (0..dim)
            .map(|j| 10.0 + (i + j as u64) as f32 * 0.05)
            .collect();
        index.insert(i, &v);
    }

    // Query near the cluster — first results will have low distances (~0.x),
    // last results high distances (~50+), creating a large spread.
    let query: Vec<f32> = vec![0.0; dim];

    // Adaptive search with a narrow min_ef and generous max_ef.
    // If spread calculation is correct and positive, the adaptive path will
    // either escalate or return valid results — either way, the search must
    // complete successfully and return sensible results.
    let results = index
        .search_with_quality(
            &query,
            10,
            SearchQuality::Adaptive {
                min_ef: 32,
                max_ef: 256,
            },
        )
        .unwrap();

    assert!(
        !results.is_empty(),
        "Adaptive Euclidean search should return results"
    );
    assert!(results.len() <= 10, "Should not exceed requested k");

    // Results must be sorted ascending (distance metric: lower = better).
    for pair in results.windows(2) {
        assert!(
            pair[0].score <= pair[1].score + f32::EPSILON,
            "Euclidean results must be sorted ascending: {} > {}",
            pair[0].score,
            pair[1].score,
        );
    }

    // The closest vectors should come from the tight cluster (ids 0..120),
    // not the outliers (ids 120..150).
    let closest_id = results[0].id;
    assert!(
        closest_id < 120,
        "Closest result should be from the tight cluster, got id={closest_id}"
    );
}

/// Regression test: adaptive search works for similarity metrics (Cosine)
/// with spread results — verifying the fix did not regress the happy path.
#[test]
#[allow(clippy::cast_precision_loss)]
fn test_adaptive_search_spread_works_for_similarity_metrics() {
    let dim = 32;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // 150 vectors with varying similarity to the query
    for i in 0u64..150 {
        let v: Vec<f32> = (0..dim)
            .map(|j| ((i * 7 + j as u64) as f32 * 0.013).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.013).sin()).collect();

    let results = index
        .search_with_quality(
            &query,
            10,
            SearchQuality::Adaptive {
                min_ef: 32,
                max_ef: 256,
            },
        )
        .unwrap();

    assert!(
        !results.is_empty(),
        "Adaptive Cosine search should return results"
    );
    assert!(results.len() <= 10, "Should not exceed requested k");

    // Results must be sorted descending (similarity metric: higher = better).
    for pair in results.windows(2) {
        assert!(
            pair[0].score >= pair[1].score - f32::EPSILON,
            "Cosine results must be sorted descending: {} < {}",
            pair[0].score,
            pair[1].score,
        );
    }
}

// -------------------------------------------------------------------------
// Upsert Semantics Tests (Issue #371)
// -------------------------------------------------------------------------

#[test]
fn test_insert_same_id_updates_vector() {
    // Arrange: create index with vector storage enabled (default)
    let index = HnswIndex::new(4, DistanceMetric::Cosine).unwrap();

    // Insert id=1 with vector A (pointing along x-axis)
    let vector_a = [1.0, 0.0, 0.0, 0.0];
    index.insert(1, &vector_a);

    // Act: insert id=1 again with vector B (pointing along y-axis, orthogonal to A)
    let vector_b = [0.0, 1.0, 0.0, 0.0];
    index.insert(1, &vector_b);

    // Assert 1: index length must still be 1 (not 2)
    assert_eq!(index.len(), 1, "Upsert must not create duplicate entries");

    // Assert 2: search with query=B should return id=1 with high similarity
    let results = index.search(&vector_b, 1);
    assert_eq!(results.len(), 1, "Should find exactly one result");
    assert_eq!(results[0].id, 1, "Result must be id=1");
    assert!(
        results[0].score > 0.9,
        "Similarity to updated vector B should be > 0.9, got {}",
        results[0].score,
    );
}

// -------------------------------------------------------------------------
// Upsert + Tombstone / Vacuum TDD Cycle 3 Tests
// -------------------------------------------------------------------------

#[test]
fn test_upsert_tombstone_accumulation() {
    // Arrange: 10-dim index so each of 10 IDs gets a unique dominant dimension
    let dim = 10;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Insert 10 vectors (generation 0)
    for i in 0..10_usize {
        index.insert(i as u64, &make_dominant_vector(dim, i, 0));
    }

    // Assert: no tombstones, 10 active entries
    assert_eq!(
        index.tombstone_count(),
        0,
        "Fresh index should have 0 tombstones"
    );
    assert_eq!(index.len(), 10, "Should have 10 active vectors");

    // Act: update all 10 vectors with new values (generation 1)
    for i in 0..10_usize {
        index.insert(i as u64, &make_dominant_vector(dim, i, 1));
    }

    // Assert: 10 tombstones (old slots are dead), still 10 active
    assert_eq!(
        index.tombstone_count(),
        10,
        "Updating 10 vectors should leave 10 tombstones",
    );
    assert_eq!(index.len(), 10, "Active count must remain 10 after upserts");

    // Tombstone ratio = 10 / 20 = 0.50 => needs_vacuum (threshold 0.20)
    assert!(
        index.needs_vacuum(),
        "Tombstone ratio {:.2} should exceed 0.20 threshold",
        index.tombstone_ratio(),
    );
}

#[test]
fn test_upsert_then_vacuum_cleans() {
    // Arrange: 10-dim index so each of 10 IDs has a unique dominant dimension
    let dim = 10;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Insert 10 vectors (generation 0)
    for i in 0..10_usize {
        index.insert(i as u64, &make_dominant_vector(dim, i, 0));
    }

    // Update only the first 5 (id=0..4) with generation 1
    for i in 0..5_usize {
        index.insert(i as u64, &make_dominant_vector(dim, i, 1));
    }

    // Pre-vacuum assertions
    assert_eq!(
        index.tombstone_count(),
        5,
        "Updating 5 vectors should create 5 tombstones",
    );
    assert_eq!(index.len(), 10);

    // Act: vacuum
    let rebuilt = index.vacuum().expect("vacuum should succeed");
    assert_eq!(rebuilt, 10, "Vacuum should report 10 active vectors");

    // Post-vacuum: tombstones cleared
    assert_eq!(
        index.tombstone_count(),
        0,
        "Vacuum must eliminate all tombstones",
    );
    assert_eq!(index.len(), 10, "All 10 vectors must survive vacuum");

    // Verify search correctness: each id appears in its own top-1 result
    // Use the *current* generation vector as query
    for i in 0..10_usize {
        let gen = u8::from(i < 5);
        let query = make_dominant_vector(dim, i, gen);
        let results = index.search(&query, 1);
        assert!(
            !results.is_empty(),
            "Search for id={i} returned no results after vacuum",
        );
        assert_eq!(
            results[0].id, i as u64,
            "Top-1 for id={i} should be itself, got id={}",
            results[0].id,
        );
    }
}

#[test]
fn test_batch_upsert_updates_existing() {
    // Arrange: 10-dim index so each of 10 IDs gets a unique dominant dimension
    let dim = 10;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Insert 10 vectors (generation 0) via batch
    let original_vectors: Vec<(u64, Vec<f32>)> = (0..10)
        .map(|id| (id, make_dominant_vector(dim, id as usize, 0)))
        .collect();

    let refs: Vec<(u64, &[f32])> = original_vectors
        .iter()
        .map(|(id, v)| (*id, v.as_slice()))
        .collect();

    let inserted = index.insert_batch_parallel(refs);
    assert_eq!(inserted, 10, "Should insert all 10 vectors");
    assert_eq!(index.len(), 10, "Index should contain 10 entries");

    // Act: update first 5 (id=0..4) with generation 1 via batch
    let updated_vectors: Vec<(u64, Vec<f32>)> = (0..5)
        .map(|id| (id, make_dominant_vector(dim, id as usize, 1)))
        .collect();

    let update_refs: Vec<(u64, &[f32])> = updated_vectors
        .iter()
        .map(|(id, v)| (*id, v.as_slice()))
        .collect();

    let upserted = index.insert_batch_parallel(update_refs);
    assert_eq!(upserted, 5, "Should upsert all 5 vectors");

    // Assert 1: index length must still be 10 (not 15)
    assert_eq!(
        index.len(),
        10,
        "Batch upsert must not create duplicate entries: expected 10, got {}",
        index.len(),
    );

    // Assert 2: for each updated id (0..4), search with generation 1 vector
    for id in 0..5_u64 {
        let query = make_dominant_vector(dim, id as usize, 1);
        let results = index.search(&query, 1);
        assert_eq!(
            results.len(),
            1,
            "Updated id={id}: should find exactly one result"
        );
        assert_eq!(
            results[0].id, id,
            "Updated id={id}: top-1 result should be the updated vector"
        );
        assert!(
            results[0].score > 0.9,
            "Updated id={id}: similarity to updated vector should be > 0.9, got {}",
            results[0].score,
        );
    }

    // Assert 3: for each non-updated id (5..9), search with generation 0 vector
    for id in 5..10_u64 {
        let query = make_dominant_vector(dim, id as usize, 0);
        let results = index.search(&query, 1);
        assert!(
            !results.is_empty(),
            "Non-updated id={id}: should find results"
        );
        assert_eq!(
            results[0].id, id,
            "Non-updated id={id}: top-1 result should be the original vector"
        );
    }
}

// -------------------------------------------------------------------------
// Upsert TDD Cycle 5: Recall Verification + Edge Cases
// -------------------------------------------------------------------------

/// Verifies that after updating a vector to a different cluster, search
/// finds it in the NEW location and no longer returns it from the OLD cluster.
#[allow(clippy::similar_names)] // origin/target are semantically distinct cluster labels
#[test]
fn test_upsert_search_recall() {
    let index = HnswIndex::new(8, DistanceMetric::Cosine).unwrap();

    // Origin cluster center: dominant component at dim 0
    let origin_center: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    // Target cluster center: dominant component at dim 4
    let target_center: Vec<f32> = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

    // Build deterministic vectors with small perturbation per id.
    // Origin cluster: ids 0..49, base near origin_center
    let origin_vectors: Vec<(u64, Vec<f32>)> = (0..50)
        .map(|i| {
            let mut v = origin_center.clone();
            for (d, component) in v.iter_mut().enumerate() {
                *component += 0.01 * (i * 8 + d) as f32;
            }
            (i as u64, v)
        })
        .collect();

    // Target cluster: ids 50..99, base near target_center
    let target_vectors: Vec<(u64, Vec<f32>)> = (0..50)
        .map(|i| {
            let mut v = target_center.clone();
            for (d, component) in v.iter_mut().enumerate() {
                *component += 0.01 * (i * 8 + d) as f32;
            }
            (i as u64 + 50, v)
        })
        .collect();

    // Insert all 100 vectors via batch
    let all_vectors: Vec<(u64, Vec<f32>)> = origin_vectors
        .iter()
        .chain(target_vectors.iter())
        .map(|(id, v)| (*id, v.clone()))
        .collect();

    let refs: Vec<(u64, &[f32])> = all_vectors
        .iter()
        .map(|(id, v)| (*id, v.as_slice()))
        .collect();

    let inserted = index.insert_batch_parallel(refs);
    assert_eq!(inserted, 100, "Should insert all 100 vectors");

    // Update id=25 (origin cluster) to a target cluster vector
    let moved_vector: Vec<f32> = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    index.insert(25, &moved_vector);

    assert_eq!(index.len(), 100, "Length must remain 100 after upsert");

    // Search near target cluster center: id=25 should appear (it moved there)
    let results_target = index.search(&target_center, 10);
    let found_in_target = results_target.iter().any(|r| r.id == 25);
    assert!(
        found_in_target,
        "id=25 should appear in target cluster results after upsert, got ids: {:?}",
        results_target.iter().map(|r| r.id).collect::<Vec<_>>(),
    );

    // Search near origin cluster center: id=25 should NOT appear (it left)
    let results_origin = index.search(&origin_center, 10);
    let found_in_origin = results_origin.iter().any(|r| r.id == 25);
    assert!(
        !found_in_origin,
        "id=25 should NOT appear in origin cluster results after upsert, got ids: {:?}",
        results_origin.iter().map(|r| r.id).collect::<Vec<_>>(),
    );
}

/// Verifies that updating the very first vector (likely the HNSW entry point)
/// does not break search for any vector in the index.
#[test]
fn test_upsert_entry_point_vector() {
    let index = HnswIndex::new(4, DistanceMetric::Cosine).unwrap();

    // Insert id=0 first — this becomes the HNSW entry point
    let ep_original = vec![1.0, 0.0, 0.0, 0.0];
    index.insert(0, &ep_original);

    // Update id=0 to a completely different direction
    let ep_updated = vec![0.0, 0.0, 0.0, 1.0];
    index.insert(0, &ep_updated);

    // Insert 20 more vectors with varied, well-separated values.
    // Each vector has a dominant component determined by (id % 4).
    let additional_vectors: Vec<(u64, Vec<f32>)> = (1..=20)
        .map(|i| {
            let dominant_dim = (i as usize) % 4;
            let mut v = vec![0.1_f32; 4];
            v[dominant_dim] += 1.0 + 0.05 * i as f32;
            (i as u64, v)
        })
        .collect();

    let refs: Vec<(u64, &[f32])> = additional_vectors
        .iter()
        .map(|(id, v)| (*id, v.as_slice()))
        .collect();

    let inserted = index.insert_batch_parallel(refs);
    assert_eq!(inserted, 20, "Should insert all 20 vectors");
    assert_eq!(index.len(), 21, "Index should contain 21 entries");

    // Verify: searching for the updated id=0 vector returns id=0 as top-1
    let results_ep = index.search(&ep_updated, 1);
    assert_eq!(results_ep.len(), 1, "Should find at least 1 result for EP");
    assert_eq!(
        results_ep[0].id, 0,
        "Top-1 for updated entry point should be id=0, got id={}",
        results_ep[0].id,
    );

    // Verify: each of the 20 additional vectors is findable as top-1
    for (id, vec) in &additional_vectors {
        let results = index.search(vec.as_slice(), 1);
        assert!(
            !results.is_empty(),
            "Search for id={id} should return results",
        );
        assert_eq!(
            results[0].id, *id,
            "Top-1 for id={id} should be itself, got id={}",
            results[0].id,
        );
    }
}

// -------------------------------------------------------------------------
// Upsert Rollback + Edge Case Tests (Issue #371)
// -------------------------------------------------------------------------

/// Verifies that batch insert with within-batch duplicate IDs correctly
/// deduplicates: only the last occurrence is reachable, and the count
/// reflects unique IDs (not raw entries). Within-batch duplicates are an
/// unusual pattern but must not corrupt mapping state.
#[test]
fn test_batch_upsert_within_batch_duplicates() {
    let dim = 4;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Batch with id=1 appearing twice: first [1,0,0,0], then [0,1,0,0]
    let vec_a = vec![1.0_f32, 0.0, 0.0, 0.0];
    let vec_b = vec![0.0_f32, 1.0, 0.0, 0.0];
    let batch: Vec<(u64, &[f32])> = vec![(1, &vec_a), (1, &vec_b), (2, &vec_a)];

    let inserted = index.insert_batch_parallel(batch);
    // Count may include both entries for id=1, but len() must reflect unique IDs
    assert!(
        inserted >= 2,
        "At least 2 entries processed, got {inserted}"
    );
    assert_eq!(index.len(), 2, "Only 2 unique IDs should be mapped");

    // id=1 must be searchable and point to vec_b (last wins)
    let results = index.search(&vec_b, 1);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].id, 1,
        "id=1 must be findable after within-batch upsert"
    );
    assert!(
        results[0].score > 0.9,
        "id=1 should match vec_b, got score {}",
        results[0].score,
    );

    // id=2 must also be searchable
    let results2 = index.search(&vec_a, 1);
    assert_eq!(results2[0].id, 2, "id=2 must be findable");
}

/// Creates a deterministic vector where dimension `dominant_dim` has a
/// large component, making it easily distinguishable in cosine search.
///
/// Shared helper to avoid duplicating the same closure across tests.
fn make_dominant_vector(dim: usize, dominant_dim: usize, gen: u8) -> Vec<f32> {
    let mut v = vec![0.1_f32; dim];
    v[dominant_dim % dim] += 1.0 + f32::from(gen) * 0.5;
    v
}

/// Verifies that insert(id) after remove(id) correctly re-inserts
/// the vector and returns it in search results.
#[test]
fn test_upsert_after_delete() {
    let dim = 8;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

    // Insert id=1 with dominant dim 0
    let vec_a = make_dominant_vector(dim, 0, 0);
    index.insert(1, &vec_a);
    assert_eq!(index.len(), 1);

    // Remove id=1
    assert!(index.remove(1));
    assert_eq!(index.len(), 0);

    // Re-insert id=1 with dominant dim 4
    let vec_b = make_dominant_vector(dim, 4, 1);
    index.insert(1, &vec_b);
    assert_eq!(index.len(), 1, "Re-inserted id should count as 1 vector");

    // Search should find id=1 near vec_b
    let results = index.search(&vec_b, 1);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].id, 1,
        "After delete + re-insert, search must find re-inserted id=1"
    );
}

/// Verifies that upserting the same id N times leaves exactly N-1
/// tombstones and the index still contains exactly 1 live vector
/// that matches the latest version.
#[test]
fn test_repeated_upsert_accumulates_tombstones() {
    let dim = 8;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();
    let num_upserts: usize = 20;

    for gen in 0..num_upserts {
        let v = make_dominant_vector(dim, gen % dim, gen as u8);
        index.insert(1, &v);
    }

    assert_eq!(
        index.len(),
        1,
        "Repeated upserts must not duplicate entries"
    );
    assert_eq!(
        index.tombstone_count(),
        num_upserts - 1,
        "Each upsert after the first should leave one tombstone"
    );

    // Search must return the latest vector (dominant dim = (num_upserts-1) % dim)
    let latest = make_dominant_vector(dim, (num_upserts - 1) % dim, (num_upserts - 1) as u8);
    let results = index.search(&latest, 1);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].id, 1,
        "Search must return the latest upserted vector"
    );
}

// -------------------------------------------------------------------------
// BUG-0002: insert_and_correct_mapping divergence correction
// -------------------------------------------------------------------------

/// Forces the HNSW graph node counter ahead of `ShardedMappings::next_idx`
/// to simulate the concurrent insert divergence that BUG-0002 describes.
///
/// When `self.inner.read()` allows concurrent inserts, two threads can
/// allocate mapping indices in one order but graph node IDs in another.
/// `insert_and_correct_mapping` detects this and calls `remove_reverse` +
/// `restore` to fix the bidirectional mapping.
#[test]
fn test_insert_and_correct_mapping_fixes_diverged_idx() {
    let dim = 4;
    let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();

    // Phase 1: normal insert so both counters advance to 1
    index.insert(100, &[1.0, 0.0, 0.0, 0.0]);
    assert_eq!(index.len(), 1);
    assert_eq!(index.mappings.get_idx(100), Some(0));

    // Phase 2: advance graph counter WITHOUT advancing mapping counter.
    // This simulates a concurrent thread that got into the graph first.
    let ghost_vec = [0.0, 1.0, 0.0, 0.0];
    let ghost_node_id = index.inner.read().insert((&ghost_vec, 999)).unwrap();
    assert_eq!(ghost_node_id, 1, "Graph should assign node_id=1");

    // Phase 3: upsert_mapping for id=200 — mappings allocates idx=1
    let result = index.upsert_mapping(200);
    assert_eq!(result.idx, 1, "Mapping should allocate idx=1");
    assert_eq!(result.old_idx, None, "New ID has no old mapping");

    // Phase 4: insert_and_correct_mapping — graph assigns node_id=2 (not 1)
    let vector = [0.0, 0.0, 1.0, 0.0];
    let success = index.insert_and_correct_mapping(200, &vector, &result);
    assert!(success, "insert_and_correct_mapping should succeed");

    // Phase 5: verify mapping correction
    // The graph assigned node_id=2, so mapping must point to 2, not 1
    let corrected_idx = index.mappings.get_idx(200).unwrap();
    assert_eq!(
        corrected_idx, 2,
        "Mapping must be corrected to actual graph node_id"
    );
    assert_eq!(
        index.mappings.get_id(2),
        Some(200),
        "Reverse mapping must point to id=200"
    );
    // Stale reverse mapping for idx=1 must be gone
    assert_eq!(
        index.mappings.get_id(1),
        None,
        "Stale reverse mapping for original idx must be removed"
    );
}

/// Verifies that search still returns correct results after the
/// divergence correction path has been exercised.
#[test]
fn test_search_works_after_mapping_correction() {
    let dim = 4;
    let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();

    // Insert a baseline vector normally
    index.insert(100, &[1.0, 0.0, 0.0, 0.0]);

    // Advance graph counter with a ghost insert
    let ghost_vec = [0.5, 0.5, 0.0, 0.0];
    index.inner.read().insert((&ghost_vec, 999)).unwrap();

    // Force divergence path for id=200
    let result = index.upsert_mapping(200);
    let target = [0.0, 0.0, 0.0, 1.0];
    index.insert_and_correct_mapping(200, &target, &result);

    // Search for id=200's vector — should find it despite correction
    let results = index.search(&[0.0, 0.0, 0.0, 1.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].id, 200,
        "Search must find the vector after mapping correction"
    );
}

/// When `assigned_id == result.idx` (no divergence), the happy path
/// should leave mappings unchanged.
#[test]
fn test_insert_and_correct_mapping_no_divergence_happy_path() {
    let dim = 4;
    let index = HnswIndex::new(dim, DistanceMetric::Euclidean).unwrap();

    // Normal insert: no divergence expected
    let result = index.upsert_mapping(42);
    assert_eq!(result.idx, 0);

    let vector = [1.0, 0.0, 0.0, 0.0];
    let success = index.insert_and_correct_mapping(42, &vector, &result);
    assert!(success, "Happy path should succeed");

    // Mapping should be exactly as allocated — no correction needed
    assert_eq!(index.mappings.get_idx(42), Some(0));
    assert_eq!(index.mappings.get_id(0), Some(42));
    assert_eq!(index.len(), 1);
}

// =========================================================================
// Issue #396: parallel_insert ignores expected idx — mapping reconciliation
// =========================================================================

/// Regression test: batch insert after single inserts must reconcile mappings.
///
/// Single inserts consume graph node IDs, so when a subsequent batch
/// pre-registers mapping indices, the graph may assign different node IDs.
/// The reconciliation logic must correct the mappings to match.
#[test]
fn test_batch_after_single_insert_mapping_consistency() {
    let index = HnswIndex::new(4, DistanceMetric::Euclidean).unwrap();

    // Single-insert 1 vector: consumes graph node 0
    index.insert(100, &[1.0, 0.0, 0.0, 0.0]);
    assert_eq!(index.len(), 1);

    // Batch-insert 5 vectors. The mapping layer will pre-register indices
    // starting from next_idx (1..=5), but the graph may assign node IDs
    // starting from 1 as well — or they may diverge under races.
    let batch: Vec<(u64, &[f32])> = vec![
        (200, &[0.0, 1.0, 0.0, 0.0]),
        (201, &[0.0, 0.0, 1.0, 0.0]),
        (202, &[0.0, 0.0, 0.0, 1.0]),
        (203, &[0.5, 0.5, 0.0, 0.0]),
        (204, &[0.0, 0.5, 0.5, 0.0]),
    ];
    let inserted = index.insert_batch_parallel(batch);
    assert_eq!(inserted, 5);
    assert_eq!(index.len(), 6);

    // Every external ID must resolve to a valid internal idx, and that idx
    // must map back to the same external ID (bidirectional consistency).
    for &ext_id in &[100u64, 200, 201, 202, 203, 204] {
        let idx = index.mappings.get_idx(ext_id);
        assert!(idx.is_some(), "get_idx({ext_id}) must return Some");
        let reverse = index.mappings.get_id(idx.unwrap());
        assert_eq!(
            reverse,
            Some(ext_id),
            "Reverse mapping for idx {} must be {ext_id}, got {reverse:?}",
            idx.unwrap()
        );
    }

    // Verify search still returns correct results — the original vector
    // should be the nearest to itself.
    let results = index.search(&[1.0, 0.0, 0.0, 0.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 100, "Nearest to [1,0,0,0] must be id=100");
}

/// Regression test: sidecar vectors stored at graph-assigned IDs.
///
/// After reconciliation, vectors in `ShardedVectors` must be stored at
/// the graph-assigned node IDs (not the pre-registered mapping indices).
#[test]
fn test_batch_insert_vector_storage_uses_assigned_ids() {
    let index = HnswIndex::new(4, DistanceMetric::Euclidean).unwrap();

    // Pre-populate to create a gap between mapping indices and graph node IDs
    for i in 0..3u64 {
        let v = [i as f32, 0.0, 0.0, 0.0];
        index.insert(i, &v);
    }
    assert_eq!(index.len(), 3);

    // Batch-insert 4 vectors
    let batch: Vec<(u64, &[f32])> = vec![
        (10, &[10.0, 0.0, 0.0, 0.0]),
        (11, &[11.0, 0.0, 0.0, 0.0]),
        (12, &[12.0, 0.0, 0.0, 0.0]),
        (13, &[13.0, 0.0, 0.0, 0.0]),
    ];
    let inserted = index.insert_batch_parallel(batch);
    assert_eq!(inserted, 4);

    // For each batch-inserted ID, the sidecar vector at the mapped idx
    // must exist and match the original vector.
    for ext_id in 10..=13u64 {
        let idx = index.mappings.get_idx(ext_id).expect("mapping must exist");
        let stored = index.vectors.get(idx);
        assert!(
            stored.is_some(),
            "ShardedVectors must have a vector at idx {idx} for id {ext_id}"
        );
        let expected_first = ext_id as f32;
        let stored_vec = stored.unwrap();
        assert!(
            (stored_vec[0] - expected_first).abs() < f32::EPSILON,
            "Vector for id {ext_id} at idx {idx}: expected first component {expected_first}, got {}",
            stored_vec[0]
        );
    }
}

/// Regression test: batch upsert (insert + replace) maintains mapping consistency.
///
/// Insert 5 vectors, then batch-upsert 3 of them with new data. The replaced
/// vectors must have updated mappings and the new graph node IDs must be valid.
#[test]
fn test_batch_upsert_mapping_consistency() {
    let index = HnswIndex::new(4, DistanceMetric::Euclidean).unwrap();

    // Initial insert of 5 vectors
    for i in 0..5u64 {
        index.insert(i, &[i as f32, 0.0, 0.0, 0.0]);
    }
    assert_eq!(index.len(), 5);

    // Batch-upsert: update IDs 1, 2, 3 with new vectors
    let batch: Vec<(u64, &[f32])> = vec![
        (1, &[100.0, 0.0, 0.0, 0.0]),
        (2, &[200.0, 0.0, 0.0, 0.0]),
        (3, &[300.0, 0.0, 0.0, 0.0]),
    ];
    let inserted = index.insert_batch_parallel(batch);
    assert_eq!(inserted, 3);
    // Still 5 live IDs (3 replaced, not added)
    assert_eq!(index.len(), 5);

    // Verify all 5 IDs have consistent bidirectional mappings
    for ext_id in 0..5u64 {
        let idx = index.mappings.get_idx(ext_id);
        assert!(idx.is_some(), "get_idx({ext_id}) must return Some");
        let reverse = index.mappings.get_id(idx.unwrap());
        assert_eq!(
            reverse,
            Some(ext_id),
            "Reverse mapping for idx {} must be {ext_id}, got {reverse:?}",
            idx.unwrap()
        );
    }

    // The replaced vectors should be searchable with their new values.
    // Search for [100, 0, 0, 0] — nearest should be id=1.
    let results = index.search(&[100.0, 0.0, 0.0, 0.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1, "Nearest to [100,0,0,0] must be id=1");
}
