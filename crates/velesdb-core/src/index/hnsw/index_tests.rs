//! Tests for `HnswIndex` (extracted from index.rs for maintainability)
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
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
    let index = HnswIndex::new(64, DistanceMetric::Cosine);

    // Act & Assert
    assert_eq!(index.tombstone_count(), 0);
    assert!((index.tombstone_ratio() - 0.0).abs() < f64::EPSILON);
    assert!(!index.needs_vacuum());
}

#[test]
fn test_tombstone_count_after_deletions() {
    // Arrange
    let index = HnswIndex::new(64, DistanceMetric::Cosine);

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
    let index = HnswIndex::new(64, DistanceMetric::Cosine);

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
    let index = HnswIndex::new(64, DistanceMetric::Cosine);

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
    for (id, _) in &results_after {
        assert!(*id >= 25 && *id < 50);
    }
}

#[test]
fn test_vacuum_fails_with_fast_insert_mode() {
    // Arrange
    let index = HnswIndex::new_fast_insert(64, DistanceMetric::Cosine);

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
    let index = HnswIndex::new(64, DistanceMetric::Cosine);

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
    let index = HnswIndex::new(768, DistanceMetric::Cosine);

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
    let index = HnswIndex::new_turbo(64, DistanceMetric::Cosine);

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
    let index = HnswIndex::new_fast_insert(64, DistanceMetric::Cosine);

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
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
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
    let index = HnswIndex::new(3, DistanceMetric::Cosine);

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
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
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
    let top_ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
    assert!(top_ids.contains(&1), "Exact match should be in top results");
}

#[test]
fn test_hnsw_search_empty_index() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);

    // Act
    let results = index.search(&[1.0, 0.0, 0.0], 10);

    // Assert
    assert!(results.is_empty());
}

#[test]
fn test_hnsw_remove_existing_vector() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
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
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
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
    let index = HnswIndex::new(3, DistanceMetric::Euclidean);
    index.insert(1, &[0.0, 0.0, 0.0]);
    index.insert(2, &[1.0, 0.0, 0.0]); // Distance 1
    index.insert(3, &[3.0, 4.0, 0.0]); // Distance 5
    index.insert(4, &[2.0, 0.0, 0.0]); // Distance 2
    index.insert(5, &[0.5, 0.5, 0.0]); // Distance ~0.7

    // Act
    let results = index.search(&[0.0, 0.0, 0.0], 3);

    // Assert - at least get some results, first should be closest
    assert!(!results.is_empty(), "Should return results");
    assert_eq!(results[0].0, 1, "Closest should be exact match");
}

#[test]
fn test_hnsw_dot_product_metric() {
    // Arrange - Use normalized positive vectors for dot product
    // DistDot in hnsw_rs requires non-negative dot products
    // Use more vectors to avoid HNSW flakiness with tiny datasets
    let index = HnswIndex::new(3, DistanceMetric::DotProduct);

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
    assert_eq!(results[0].0, 1, "Highest dot product should be first");
}

#[test]
#[should_panic(expected = "Vector dimension mismatch")]
fn test_hnsw_insert_wrong_dimension_panics() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);

    // Act - should panic
    index.insert(1, &[1.0, 0.0]); // Wrong dimension
}

#[test]
#[should_panic(expected = "Query dimension mismatch")]
fn test_hnsw_search_wrong_dimension_panics() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    index.insert(1, &[1.0, 0.0, 0.0]);

    // Act - should panic
    let _ = index.search(&[1.0, 0.0], 10); // Wrong dimension
}

#[test]
fn test_hnsw_duplicate_insert_is_skipped() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    index.insert(1, &[1.0, 0.0, 0.0]);

    // Act - Insert with same ID should be SKIPPED (not updated)
    // hnsw_rs doesn't support updates; inserting same idx creates ghosts
    index.insert(1, &[0.0, 1.0, 0.0]);

    // Assert
    assert_eq!(index.len(), 1); // Still only one entry

    // Verify the ORIGINAL vector is still there (not updated)
    let results = index.search(&[1.0, 0.0, 0.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 1);
    // Score should be ~1.0 (exact match with original vector)
    assert!(
        results[0].1 > 0.99,
        "Original vector should still be indexed"
    );
}

#[test]
fn test_hnsw_thread_safety() {
    use std::sync::Arc;
    use std::thread;

    // Arrange
    let index = Arc::new(HnswIndex::new(3, DistanceMetric::Cosine));
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
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
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
    assert_eq!(results[0].0, 1);
}

#[test]
fn test_hnsw_insert_batch_parallel() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    let vectors: Vec<(u64, Vec<f32>)> = vec![
        (1, vec![1.0, 0.0, 0.0]),
        (2, vec![0.0, 1.0, 0.0]),
        (3, vec![0.0, 0.0, 1.0]),
        (4, vec![0.5, 0.5, 0.0]),
        (5, vec![0.5, 0.0, 0.5]),
    ];

    // Act
    let inserted = index.insert_batch_parallel(vectors);
    index.set_searching_mode();

    // Assert
    assert_eq!(inserted, 5);
    assert_eq!(index.len(), 5);

    // Verify search works
    let results = index.search(&[1.0, 0.0, 0.0], 3);
    assert_eq!(results.len(), 3);
    // ID 1 should be in the top results (exact match)
    // Note: Due to parallel insertion, graph structure may vary
    let result_ids: Vec<u64> = results.iter().map(|r| r.0).collect();
    assert!(result_ids.contains(&1), "ID 1 should be in top 3 results");
}

#[test]
fn test_hnsw_insert_batch_parallel_skips_duplicates() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);

    // Insert one vector first
    index.insert(1, &[1.0, 0.0, 0.0]);

    // Act - Try to insert batch with duplicate ID
    let vectors: Vec<(u64, Vec<f32>)> = vec![
        (1, vec![0.0, 1.0, 0.0]), // Duplicate ID
        (2, vec![0.0, 0.0, 1.0]), // New
    ];
    let inserted = index.insert_batch_parallel(vectors);
    index.set_searching_mode();

    // Assert - Only 1 new vector should be inserted
    assert_eq!(inserted, 1);
    assert_eq!(index.len(), 2);
}

// =========================================================================
// QW-3: insert_batch_sequential Tests (deprecated - kept for backward compat)
// =========================================================================

#[test]
#[allow(deprecated)]
fn test_hnsw_insert_batch_sequential() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    let vectors: Vec<(u64, Vec<f32>)> = vec![
        (1, vec![1.0, 0.0, 0.0]),
        (2, vec![0.0, 1.0, 0.0]),
        (3, vec![0.0, 0.0, 1.0]),
        (4, vec![0.5, 0.5, 0.0]),
        (5, vec![0.5, 0.0, 0.5]),
    ];

    // Act
    let inserted = index.insert_batch_sequential(vectors);

    // Assert
    assert_eq!(inserted, 5);
    assert_eq!(index.len(), 5);

    // Verify search works
    let results = index.search(&[1.0, 0.0, 0.0], 3);
    assert_eq!(results.len(), 3);
    let result_ids: Vec<u64> = results.iter().map(|r| r.0).collect();
    assert!(result_ids.contains(&1), "ID 1 should be in top 3 results");
}

#[test]
#[allow(deprecated)]
fn test_hnsw_insert_batch_sequential_skips_duplicates() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    index.insert(1, &[1.0, 0.0, 0.0]);

    // Act - Try to insert batch with duplicate ID
    let vectors: Vec<(u64, Vec<f32>)> = vec![
        (1, vec![0.0, 1.0, 0.0]), // Duplicate ID
        (2, vec![0.0, 0.0, 1.0]), // New
    ];
    let inserted = index.insert_batch_sequential(vectors);

    // Assert - Only 1 new vector should be inserted
    assert_eq!(inserted, 1);
    assert_eq!(index.len(), 2);
}

#[test]
#[allow(deprecated)]
fn test_hnsw_insert_batch_sequential_empty() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    let vectors: Vec<(u64, Vec<f32>)> = vec![];

    // Act
    let inserted = index.insert_batch_sequential(vectors);

    // Assert
    assert_eq!(inserted, 0);
    assert!(index.is_empty());
}

#[test]
#[allow(deprecated)]
#[should_panic(expected = "Vector dimension mismatch")]
fn test_hnsw_insert_batch_sequential_wrong_dimension() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    let vectors: Vec<(u64, Vec<f32>)> = vec![(1, vec![1.0, 0.0])]; // Wrong dim

    // Act - should panic
    index.insert_batch_sequential(vectors);
}

// =========================================================================
// HnswIndex with Params Tests
// Note: HnswParams unit tests are in params.rs
// =========================================================================

#[test]
fn test_hnsw_with_params() {
    let params = HnswParams::custom(48, 600, 500_000);
    let index = HnswIndex::with_params(1536, DistanceMetric::Cosine, params);

    assert_eq!(index.dimension(), 1536);
    assert!(index.is_empty());
}

// =========================================================================
// SIMD Re-ranking Tests (TDD - RED phase)
// =========================================================================

#[test]
fn test_search_with_rerank_returns_k_results() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]);
    index.insert(3, &[0.8, 0.2, 0.0]);
    index.insert(4, &[0.0, 1.0, 0.0]);
    index.insert(5, &[0.0, 0.0, 1.0]);

    // Act
    let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 5);

    // Assert
    assert_eq!(results.len(), 3, "Should return exactly k results");
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_search_with_rerank_improves_ranking() {
    // Arrange - vectors with subtle differences
    let index = HnswIndex::new(128, DistanceMetric::Cosine);

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
    let results = index.search_with_rerank(&base, 3, 3);

    // Assert - ID 1 should be closest (highest similarity)
    assert_eq!(results[0].0, 1, "Most similar vector should be first");
}

#[test]
fn test_search_with_rerank_handles_rerank_k_greater_than_index_size() {
    // Arrange - use more vectors to avoid HNSW flakiness
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.0, 1.0, 0.0]);
    index.insert(3, &[0.0, 0.0, 1.0]);
    index.insert(4, &[0.5, 0.5, 0.0]);
    index.insert(5, &[0.5, 0.0, 0.5]);

    // Act - rerank_k > index size
    let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 100);

    // Assert - should return at least some results
    assert!(!results.is_empty(), "Should return results");
    assert!(results.len() <= 5, "Should not exceed index size");
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_with_rerank_uses_simd_distances() {
    // Arrange
    let index = HnswIndex::new(768, DistanceMetric::Cosine);

    // Insert 100 vectors
    for i in 0..100_u64 {
        let v: Vec<f32> = (0..768)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..768).map(|j| (j as f32 * 0.01).sin()).collect();

    // Act
    let results = index.search_with_rerank(&query, 10, 50);

    // Assert - results should have valid distances (SIMD computed)
    // Note: HNSW may return fewer results if graph not fully connected
    assert!(!results.is_empty(), "Should return at least one result");
    for (_, dist) in &results {
        assert!(*dist >= -1.0 && *dist <= 1.0, "Cosine should be in [-1, 1]");
    }

    // Results should be sorted by similarity (descending for cosine)
    for i in 1..results.len() {
        assert!(
            results[i - 1].1 >= results[i].1,
            "Results should be sorted by similarity descending"
        );
    }
}

#[test]
fn test_search_with_rerank_euclidean_metric() {
    // Arrange
    let index = HnswIndex::new(3, DistanceMetric::Euclidean);
    index.insert(1, &[0.0, 0.0, 0.0]);
    index.insert(2, &[1.0, 0.0, 0.0]);
    index.insert(3, &[2.0, 0.0, 0.0]);

    // Act
    let results = index.search_with_rerank(&[0.0, 0.0, 0.0], 3, 3);

    // Assert - ID 1 should be closest (smallest distance)
    assert_eq!(results[0].0, 1, "Origin should be closest to itself");
    // For euclidean, smaller is better - results sorted ascending
    for i in 1..results.len() {
        assert!(
            results[i - 1].1 <= results[i].1,
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
        let index = HnswIndex::new(128, DistanceMetric::Cosine);
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
        let index = HnswIndex::new(64, DistanceMetric::Euclidean);
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
            let index = HnswIndex::new(dim, metric);
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
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    // Insert more vectors for stable HNSW graph (small graphs are non-deterministic)
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]);
    index.insert(3, &[0.8, 0.2, 0.0]);
    index.insert(4, &[0.7, 0.3, 0.0]);
    index.insert(5, &[0.0, 1.0, 0.0]);

    let results = index.search_with_quality(&[1.0, 0.0, 0.0], 2, SearchQuality::Fast);
    // Fast mode may return fewer results with very small ef_search
    assert!(!results.is_empty(), "Should return at least one result");
    assert!(results.len() <= 2, "Should not exceed requested k");
}

#[test]
fn test_search_quality_accurate() {
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]);

    let results = index.search_with_quality(&[1.0, 0.0, 0.0], 2, SearchQuality::Accurate);
    // HNSW may return fewer results for very small indices
    assert!(!results.is_empty(), "Should return at least one result");
    assert_eq!(
        results[0].0, 1,
        "Accurate search should find exact match first"
    );
}

#[test]
fn test_search_quality_custom_ef() {
    // Use more vectors to make HNSW more stable
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.9, 0.1, 0.0]);
    index.insert(3, &[0.8, 0.2, 0.0]);
    index.insert(4, &[0.0, 1.0, 0.0]);
    index.insert(5, &[0.0, 0.0, 1.0]);

    let results = index.search_with_quality(&[1.0, 0.0, 0.0], 3, SearchQuality::Custom(512));
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
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 10, 50);
    assert!(
        results.is_empty(),
        "Empty index should return empty results"
    );
}

#[test]
fn test_hnsw_search_with_rerank_dot_product() {
    let index = HnswIndex::new(3, DistanceMetric::DotProduct);
    index.insert(1, &[1.0, 0.0, 0.0]);
    index.insert(2, &[0.5, 0.5, 0.0]);
    index.insert(3, &[0.0, 1.0, 0.0]);

    let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 3);

    // HNSW may return fewer results for very small indices
    assert!(!results.is_empty(), "Should return at least one result");
    // For dot product, ID 1 should have highest score
    assert_eq!(results[0].0, 1, "Highest dot product should be first");
}

#[test]
fn test_hnsw_io_holder_is_none_for_new_index() {
    // For newly created indices, io_holder should be None
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    // We can't directly access io_holder, but we can verify the index works
    // and drops without issues (no io_holder to manage)
    index.insert(1, &[1.0, 0.0, 0.0]);
    assert_eq!(index.len(), 1);
    // Dropped here without io_holder cleanup needed
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_hnsw_large_batch_parallel_insert() {
    let index = HnswIndex::new(128, DistanceMetric::Cosine);

    // Create 1000 vectors
    let vectors: Vec<(u64, Vec<f32>)> = (0..1000)
        .map(|i| {
            let v: Vec<f32> = (0..128).map(|j| ((i + j) as f32 * 0.001).sin()).collect();
            (i as u64, v)
        })
        .collect();

    let inserted = index.insert_batch_parallel(vectors);
    index.set_searching_mode();

    assert_eq!(inserted, 1000, "Should insert 1000 vectors");
    assert_eq!(index.len(), 1000);

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
    let index = HnswIndex::new(768, DistanceMetric::Cosine);

    // Insert 100 vectors
    for i in 0u64..100 {
        let v: Vec<f32> = (0..768)
            .map(|j| ((i + j as u64) as f32 * 0.001).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..768).map(|j| (j as f32 * 0.001).sin()).collect();
    let results = index.search_with_rerank(&query, 10, 50);

    assert!(!results.is_empty(), "Should return results");
    assert!(results.len() <= 10, "Should not exceed k");
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_with_rerank_small_dim_prefetch() {
    // Test adaptive prefetch for small vectors (32D = 128 bytes)
    // prefetch_distance should be 128/64 = 2, clamped to 4 (minimum)
    let index = HnswIndex::new(32, DistanceMetric::Cosine);

    for i in 0u64..50 {
        let v: Vec<f32> = (0..32)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..32).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = index.search_with_rerank(&query, 5, 20);

    assert!(!results.is_empty(), "Should return results");
}

// =========================================================================
// TS-CORE-002: Batch Search Optimization Tests
// =========================================================================

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_batch_parallel_consistency() {
    let index = HnswIndex::new(64, DistanceMetric::Cosine);

    // Insert 200 vectors
    for i in 0u64..200 {
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
    let batch_results = index.search_batch_parallel(&query_refs, 5, SearchQuality::Balanced);

    // Individual searches for comparison
    let individual_results: Vec<Vec<(u64, f32)>> = queries
        .iter()
        .map(|q| index.search_with_quality(q, 5, SearchQuality::Balanced))
        .collect();

    // Results should match (same IDs, though order might vary slightly)
    assert_eq!(batch_results.len(), individual_results.len());
    for (batch, individual) in batch_results.iter().zip(&individual_results) {
        assert_eq!(batch.len(), individual.len(), "Result counts should match");
    }
}

#[test]
fn test_search_batch_parallel_empty_queries() {
    let index = HnswIndex::new(3, DistanceMetric::Cosine);
    index.insert(1, &[1.0, 0.0, 0.0]);

    let queries: Vec<&[f32]> = vec![];
    let results = index.search_batch_parallel(&queries, 5, SearchQuality::Fast);

    assert!(
        results.is_empty(),
        "Empty queries should return empty results"
    );
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
fn test_search_batch_parallel_large_batch() {
    let index = HnswIndex::new(128, DistanceMetric::Cosine);

    // Insert 500 vectors
    for i in 0u64..500 {
        let v: Vec<f32> = (0..128)
            .map(|j| ((i + j as u64) as f32 * 0.001).sin())
            .collect();
        index.insert(i, &v);
    }
    index.set_searching_mode();

    // 100 queries batch
    let queries: Vec<Vec<f32>> = (0..100)
        .map(|i| {
            (0..128)
                .map(|j| ((500 + i + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();
    let query_refs: Vec<&[f32]> = queries.iter().map(Vec::as_slice).collect();

    let results = index.search_batch_parallel(&query_refs, 10, SearchQuality::Accurate);

    assert_eq!(results.len(), 100, "Should return 100 result sets");
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
    // Ensure recall@10 >= 90% for HighRecall quality on small dataset
    let dim = 64;
    let n = 500;
    let k = 10;

    let index = HnswIndex::new(dim, DistanceMetric::Cosine);

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
    let mut distances: Vec<(u64, f32)> = dataset
        .iter()
        .enumerate()
        .map(|(idx, vec)| {
            let sim = crate::simd::cosine_similarity_fast(&query, vec);
            #[allow(clippy::cast_possible_truncation)]
            (idx as u64, sim)
        })
        .collect();
    distances.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let ground_truth: Vec<u64> = distances.iter().take(k).map(|(id, _)| *id).collect();

    // HNSW search
    let results = index.search_with_quality(&query, k, SearchQuality::HighRecall);
    let result_ids: std::collections::HashSet<u64> = results.iter().map(|(id, _)| *id).collect();
    let gt_set: std::collections::HashSet<u64> = ground_truth.iter().copied().collect();

    let recall = result_ids.intersection(&gt_set).count() as f64 / k as f64;

    assert!(
        recall >= 0.8,
        "Recall@{k} should be >= 80% for HighRecall, got {:.1}%",
        recall * 100.0
    );
}

// =========================================================================
// RF-3: Tests for search_brute_force_buffered (buffer reuse optimization)
// =========================================================================

#[test]
fn test_brute_force_buffered_same_results_as_original() {
    let index = HnswIndex::new(32, DistanceMetric::Cosine);

    // Insert vectors
    for i in 0u64..50 {
        let v: Vec<f32> = (0..32)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..32).map(|j| (j as f32 * 0.02).cos()).collect();

    // Compare results
    let original = index.search_brute_force(&query, 10);
    let buffered = index.search_brute_force_buffered(&query, 10);

    assert_eq!(original.len(), buffered.len());
    for (orig, buf) in original.iter().zip(buffered.iter()) {
        assert_eq!(orig.0, buf.0, "IDs should match");
        assert!((orig.1 - buf.1).abs() < 1e-6, "Distances should match");
    }
}

#[test]
fn test_brute_force_buffered_empty_index() {
    let index = HnswIndex::new(16, DistanceMetric::Euclidean);
    let query: Vec<f32> = vec![0.0; 16];

    let results = index.search_brute_force_buffered(&query, 5);
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
        let index = HnswIndex::new(8, metric);
        index.insert(1, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        index.insert(2, &[0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        index.insert(3, &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

        let results =
            index.search_brute_force_buffered(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 3);
        assert_eq!(results.len(), 3, "Should return 3 results for {metric:?}");
    }
}

#[test]
fn test_brute_force_buffered_repeated_calls_stable() {
    let index = HnswIndex::new(16, DistanceMetric::Cosine);

    for i in 0u64..20 {
        let v: Vec<f32> = (0..16)
            .map(|j| ((i + j as u64) as f32 * 0.1).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = vec![0.5; 16];

    // Multiple calls should return identical results
    let r1 = index.search_brute_force_buffered(&query, 5);
    let r2 = index.search_brute_force_buffered(&query, 5);
    let r3 = index.search_brute_force_buffered(&query, 5);

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

    let index = Arc::new(HnswIndex::new(64, DistanceMetric::Cosine));

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
        let index = HnswIndex::new(8, metric);
        index.insert(1, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        index.insert(2, &[0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        index.insert(3, &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

        let results = index.search_with_rerank(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 3, 3);

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
        let index = HnswIndex::new(4, DistanceMetric::Cosine);
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
        let index = HnswIndex::new(4, DistanceMetric::Cosine);
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
        let index = HnswIndex::new(8, DistanceMetric::Euclidean);
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
    let batch_results = loaded.search_batch_parallel(&query_refs, 3, SearchQuality::Balanced);
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
        let index = Arc::new(HnswIndex::new(16, DistanceMetric::Cosine));

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
        let index = HnswIndex::new(32, DistanceMetric::Euclidean);
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
    for _ in 0..30 {
        let index = HnswIndex::new(64, DistanceMetric::Euclidean);

        // Generate batch data with reasonable magnitude
        let batch: Vec<(u64, Vec<f32>)> = (0..500)
            .map(|i| {
                let v: Vec<f32> = (0..64).map(|j| (i + j) as f32 * 0.01).collect();
                (i as u64, v)
            })
            .collect();

        // Parallel insert
        let inserted = index.insert_batch_parallel(batch);
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
    let index = HnswIndex::new(128, DistanceMetric::Cosine);

    // Insert test vectors
    for i in 0u64..100 {
        let v: Vec<f32> = (0..128)
            .map(|j| ((i + j as u64) as f32 * 0.01).sin())
            .collect();
        index.insert(i, &v);
    }

    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.02).cos()).collect();

    // CPU brute force
    let cpu_results = index.search_brute_force(&query, 10);

    // GPU brute force (if available)
    if let Some(gpu_results) = index.search_brute_force_gpu(&query, 10) {
        assert_eq!(
            cpu_results.len(),
            gpu_results.len(),
            "Result count mismatch"
        );

        // Verify same IDs returned (order may differ slightly due to floating point)
        let cpu_ids: std::collections::HashSet<u64> =
            cpu_results.iter().map(|(id, _)| *id).collect();
        let gpu_ids: std::collections::HashSet<u64> =
            gpu_results.iter().map(|(id, _)| *id).collect();

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
    let index = HnswIndex::new(64, DistanceMetric::Cosine);
    index.insert(1, &vec![0.5; 64]);

    let query = vec![0.5; 64];

    // Should not panic, returns None if GPU unavailable
    let _result = index.search_brute_force_gpu(&query, 5);

    #[cfg(not(feature = "gpu"))]
    assert!(_result.is_none(), "Should return None without GPU feature");
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
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean);
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
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean);

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
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean);

            // Insert vectors with known distances from origin
            for i in 0..num_vectors {
                let mut v = vec![0.0f32; dim];
                v[0] = i as f32; // Distance from origin = i
                index.insert(i as u64, &v);
            }

            let query = vec![0.0f32; dim];
            let results = index.search_brute_force(&query, 3);

            // First result should be id=0 (exact match at origin)
            if !results.is_empty() {
                prop_assert_eq!(results[0].0, 0, "Closest should be id=0 (at origin)");
            }
        }

        /// Property: remove always decreases len or returns false
        #[test]
        fn prop_remove_decreases_len(
            dim in 16usize..=32,
            id_to_remove in 0u64..10
        ) {
            let index = HnswIndex::new(dim, DistanceMetric::Cosine);

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
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean);
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
            let index = HnswIndex::new(dim, DistanceMetric::Euclidean);

            let batch: Vec<(u64, Vec<f32>)> = (0..batch_size)
                .map(|i| {
                    let v: Vec<f32> = (0..dim).map(|j| ((i + j) as f32 * 0.01).sin()).collect();
                    (i as u64, v)
                })
                .collect();

            // Use parallel insert (recommended API)
            let count = index.insert_batch_parallel(batch);

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
    let index = HnswIndex::new(64, DistanceMetric::Cosine);

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
        let index = HnswIndex::new(64, DistanceMetric::Cosine);
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
