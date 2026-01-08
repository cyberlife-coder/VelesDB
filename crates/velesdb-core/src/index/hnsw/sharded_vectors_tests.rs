//! Tests for `ShardedVectors` (extracted for maintainability)
#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use super::sharded_vectors::{ShardedVectors, NUM_SHARDS};
use std::sync::Arc;
use std::thread;

// -------------------------------------------------------------------------
// Basic functionality tests
// -------------------------------------------------------------------------

#[test]
fn test_sharded_vectors_new_is_empty() {
    // Arrange & Act
    let storage = ShardedVectors::new(3);

    // Assert
    assert!(storage.is_empty());
    assert_eq!(storage.len(), 0);
}

#[test]
fn test_sharded_vectors_insert_and_get() {
    // Arrange
    let storage = ShardedVectors::new(3);
    let vector = vec![1.0, 2.0, 3.0];

    // Act
    storage.insert(0, &vector);

    // Assert
    assert_eq!(storage.get(0), Some(vector));
    assert_eq!(storage.len(), 1);
}

#[test]
fn test_sharded_vectors_insert_multiple_shards() {
    // Arrange
    let storage = ShardedVectors::new(3);

    // Act - insert vectors that should go to different shards
    for i in 0..32 {
        #[allow(clippy::cast_precision_loss)]
        let val = i as f32;
        storage.insert(i, &[val; 3]);
    }

    // Assert
    assert_eq!(storage.len(), 32);
    for i in 0..32 {
        #[allow(clippy::cast_precision_loss)]
        let val = i as f32;
        assert_eq!(storage.get(i), Some(vec![val; 3]));
    }
}

#[test]
fn test_sharded_vectors_get_nonexistent() {
    // Arrange
    let storage = ShardedVectors::new(3);

    // Act & Assert
    assert_eq!(storage.get(999), None);
}

#[test]
fn test_sharded_vectors_contains() {
    // Arrange
    let storage = ShardedVectors::new(1);
    storage.insert(42, &[1.0]);

    // Act & Assert
    assert!(storage.contains(42));
    assert!(!storage.contains(999));
}

#[test]
fn test_sharded_vectors_remove() {
    // Arrange
    let storage = ShardedVectors::new(2);
    storage.insert(42, &[1.0, 2.0]);

    // Act
    let removed = storage.remove(42);

    // Assert
    assert_eq!(removed, Some(vec![1.0, 2.0]));
    assert!(!storage.contains(42));
    assert!(storage.is_empty());
}

#[test]
fn test_sharded_vectors_remove_nonexistent() {
    // Arrange
    let storage = ShardedVectors::new(1);

    // Act & Assert
    assert_eq!(storage.remove(999), None);
}

#[test]
fn test_sharded_vectors_with_vector() {
    // Arrange
    let storage = ShardedVectors::new(3);
    storage.insert(0, &[1.0, 2.0, 3.0]);

    // Act
    let sum = storage.with_vector(0, |v| v.iter().sum::<f32>());

    // Assert
    assert_eq!(sum, Some(6.0));
}

#[test]
fn test_sharded_vectors_with_vector_nonexistent() {
    // Arrange
    let storage = ShardedVectors::new(1);

    // Act
    let result = storage.with_vector(999, <[f32]>::len);

    // Assert
    assert_eq!(result, None);
}

#[test]
fn test_sharded_vectors_insert_batch() {
    // Arrange
    let storage = ShardedVectors::new(3);
    #[allow(clippy::cast_precision_loss)]
    let batch: Vec<(usize, Vec<f32>)> = (0..100).map(|i| (i, vec![i as f32; 3])).collect();

    // Act
    storage.insert_batch(batch);

    // Assert
    assert_eq!(storage.len(), 100);
    for i in 0..100 {
        #[allow(clippy::cast_precision_loss)]
        let val = i as f32;
        assert_eq!(storage.get(i), Some(vec![val; 3]));
    }
}

#[test]
fn test_sharded_vectors_iter_all() {
    // Arrange
    let storage = ShardedVectors::new(1);
    storage.insert(0, &[1.0]);
    storage.insert(16, &[2.0]); // Same shard as 0
    storage.insert(1, &[3.0]); // Different shard

    // Act
    let all: Vec<(usize, Vec<f32>)> = storage.iter_all();

    // Assert
    assert_eq!(all.len(), 3);
}

#[test]
fn test_sharded_vectors_for_each_parallel() {
    // Arrange
    let storage = ShardedVectors::new(1);
    for i in 0..50 {
        #[allow(clippy::cast_precision_loss)]
        let val = i as f32;
        storage.insert(i, &[val]);
    }

    // Act
    let mut sum = 0.0;
    storage.for_each_parallel(|_, v| {
        sum += v[0];
    });

    // Assert - sum of 0..50 = 1225
    assert!((sum - 1225.0).abs() < f32::EPSILON);
}

// -------------------------------------------------------------------------
// Shard distribution tests
// -------------------------------------------------------------------------

#[test]
fn test_shard_index_distribution() {
    // Verify that shard_index distributes evenly
    for i in 0..NUM_SHARDS {
        assert_eq!(ShardedVectors::shard_index(i), i);
    }
    // Wraparound
    assert_eq!(ShardedVectors::shard_index(16), 0);
    assert_eq!(ShardedVectors::shard_index(17), 1);
    assert_eq!(ShardedVectors::shard_index(32), 0);
}

// -------------------------------------------------------------------------
// Concurrency tests - Critical for EPIC-A validation
// -------------------------------------------------------------------------

#[test]
fn test_sharded_vectors_concurrent_insert() {
    // Arrange
    let storage = Arc::new(ShardedVectors::new(768));
    let num_threads = 8;
    let vectors_per_thread = 1000;

    // Act - spawn threads that insert unique vectors
    let handles: Vec<_> = (0..num_threads)
        .map(|t| {
            let s = Arc::clone(&storage);
            thread::spawn(move || {
                let start = t * vectors_per_thread;
                for i in start..(start + vectors_per_thread) {
                    s.insert(i, &[i as f32; 768]);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("Thread should not panic");
    }

    // Assert
    assert_eq!(storage.len(), num_threads * vectors_per_thread);
}

#[test]
fn test_sharded_vectors_concurrent_read_write() {
    // Arrange
    let storage = Arc::new(ShardedVectors::new(128));

    // Pre-populate
    for i in 0..1000 {
        storage.insert(i, &[i as f32; 128]);
    }

    let num_readers = 4;
    let num_writers = 4;

    // Act
    let mut handles = vec![];

    // Readers
    for _ in 0..num_readers {
        let s = Arc::clone(&storage);
        handles.push(thread::spawn(move || {
            for _ in 0..10000 {
                let _ = s.get(500);
                let _ = s.contains(500);
                let _ = s.with_vector(500, <[f32]>::len);
            }
        }));
    }

    // Writers
    for t in 0..num_writers {
        let s = Arc::clone(&storage);
        handles.push(thread::spawn(move || {
            let start = 1000 + t * 100;
            for i in start..(start + 100) {
                s.insert(i, &[i as f32; 128]);
            }
        }));
    }

    // Assert - no deadlocks, no panics
    for h in handles {
        h.join().expect("Thread should not panic");
    }

    assert_eq!(storage.len(), 1000 + num_writers * 100);
}

#[test]
fn test_sharded_vectors_parallel_batch_insert() {
    // Arrange
    let storage = Arc::new(ShardedVectors::new(64));
    let num_threads = 4;
    let batch_size = 250;

    // Act - each thread inserts a batch
    let handles: Vec<_> = (0..num_threads)
        .map(|t| {
            let s = Arc::clone(&storage);
            thread::spawn(move || {
                let start = t * batch_size;
                let batch: Vec<(usize, Vec<f32>)> = (start..(start + batch_size))
                    .map(|i| (i, vec![i as f32; 64]))
                    .collect();
                s.insert_batch(batch);
            })
        })
        .collect();

    for h in handles {
        h.join().expect("Thread should not panic");
    }

    // Assert
    assert_eq!(storage.len(), num_threads * batch_size);
}

#[test]
fn test_sharded_vectors_no_data_corruption() {
    // Verify that concurrent operations don't corrupt data
    let storage = Arc::new(ShardedVectors::new(10));
    let num_threads = 8;
    let ops_per_thread = 500;

    let handles: Vec<_> = (0..num_threads)
        .map(|t| {
            let s = Arc::clone(&storage);
            thread::spawn(move || {
                for i in 0..ops_per_thread {
                    let idx = t * ops_per_thread + i;
                    let expected = vec![idx as f32; 10];
                    s.insert(idx, &expected);

                    // Verify immediately
                    let retrieved = s.get(idx);
                    assert_eq!(retrieved, Some(expected), "Data corruption at idx {idx}");
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("No data corruption");
    }

    // Final verification
    for idx in 0..(num_threads * ops_per_thread) {
        let expected = vec![idx as f32; 10];
        assert_eq!(storage.get(idx), Some(expected));
    }
}

// -------------------------------------------------------------------------
// TDD: par_iter_all() tests for rayon support (EPIC-A.2)
// -------------------------------------------------------------------------

#[test]
fn test_sharded_vectors_collect_for_parallel_returns_all() {
    // Arrange
    let storage = ShardedVectors::new(4);
    for i in 0..100 {
        storage.insert(i, &[i as f32; 4]);
    }

    // Act
    let collected = storage.collect_for_parallel();

    // Assert
    assert_eq!(collected.len(), 100);
    for (idx, vec) in &collected {
        assert_eq!(vec.len(), 4);
        assert_eq!(vec[0], *idx as f32);
    }
}

#[test]
fn test_sharded_vectors_collect_for_parallel_empty() {
    // Arrange
    let storage = ShardedVectors::new(4);

    // Act
    let collected = storage.collect_for_parallel();

    // Assert
    assert!(collected.is_empty());
}

#[test]
fn test_sharded_vectors_par_map_computes_correctly() {
    use rayon::prelude::*;

    // Arrange
    let storage = ShardedVectors::new(4);
    for i in 0..50 {
        storage.insert(i, &[i as f32; 4]);
    }

    // Act - Use collect_for_parallel with rayon par_iter
    let results: Vec<(usize, f32)> = storage
        .collect_for_parallel()
        .par_iter()
        .map(|(idx, vec)| (*idx, vec.iter().sum::<f32>()))
        .collect();

    // Assert
    assert_eq!(results.len(), 50);
    for (idx, sum) in &results {
        // Sum of 4 elements of value idx
        assert_eq!(*sum, *idx as f32 * 4.0);
    }
}

#[test]
fn test_sharded_vectors_par_filter_map_works() {
    use rayon::prelude::*;

    // Arrange
    let storage = ShardedVectors::new(4);
    for i in 0..100 {
        storage.insert(i, &[i as f32; 4]);
    }

    // Act - Filter only even indices
    let results: Vec<usize> = storage
        .collect_for_parallel()
        .par_iter()
        .filter_map(|(idx, _)| if *idx % 2 == 0 { Some(*idx) } else { None })
        .collect();

    // Assert
    assert_eq!(results.len(), 50);
    for idx in &results {
        assert_eq!(*idx % 2, 0);
    }
}

// =========================================================================
// RF-3: TDD Tests for collect_into (buffer reuse optimization)
// =========================================================================

#[test]
fn test_collect_into_reuses_buffer() {
    // Arrange
    let storage = ShardedVectors::new(4);
    for i in 0..50 {
        storage.insert(i, &[i as f32; 4]);
    }

    // Act - First collection
    let mut buffer: Vec<(usize, Vec<f32>)> = Vec::with_capacity(100);
    storage.collect_into(&mut buffer);

    // Assert
    assert_eq!(buffer.len(), 50);
    assert!(buffer.capacity() >= 100); // Capacity preserved

    // Act - Second collection (reuse buffer)
    buffer.clear();
    storage.collect_into(&mut buffer);

    // Assert - Buffer reused, no reallocation
    assert_eq!(buffer.len(), 50);
    assert!(buffer.capacity() >= 100);
}

#[test]
fn test_collect_into_clears_and_fills() {
    // Arrange
    let storage = ShardedVectors::new(3);
    for i in 0..20 {
        storage.insert(i, &[i as f32; 3]);
    }

    // Pre-fill buffer with garbage
    let mut buffer: Vec<(usize, Vec<f32>)> = vec![(999, vec![0.0; 3]); 5];

    // Act
    storage.collect_into(&mut buffer);

    // Assert - Buffer cleared and filled with storage content
    assert_eq!(buffer.len(), 20);
    assert!(!buffer.iter().any(|(idx, _)| *idx == 999));
}

#[test]
fn test_collect_into_empty_storage() {
    // Arrange
    let storage = ShardedVectors::new(1);
    let mut buffer: Vec<(usize, Vec<f32>)> = vec![(1, vec![1.0]); 10];

    // Act
    storage.collect_into(&mut buffer);

    // Assert
    assert!(buffer.is_empty());
}

#[test]
fn test_collect_into_matches_collect_for_parallel() {
    // Arrange
    let storage = ShardedVectors::new(8);
    for i in 0..100 {
        storage.insert(i, &[i as f32; 8]);
    }

    // Act
    let collected = storage.collect_for_parallel();
    let mut buffer = Vec::new();
    storage.collect_into(&mut buffer);

    // Assert - Same content (order may differ due to sharding)
    assert_eq!(collected.len(), buffer.len());

    let mut collected_sorted: Vec<_> = collected.iter().map(|(idx, _)| *idx).collect();
    let mut buffer_sorted: Vec<_> = buffer.iter().map(|(idx, _)| *idx).collect();
    collected_sorted.sort_unstable();
    buffer_sorted.sort_unstable();

    assert_eq!(collected_sorted, buffer_sorted);
}
