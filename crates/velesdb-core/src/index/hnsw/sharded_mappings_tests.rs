//! Tests for `sharded_mappings` module

use super::sharded_mappings::*;
use std::sync::Arc;
use std::thread;

// -------------------------------------------------------------------------
// Basic functionality tests
// -------------------------------------------------------------------------

#[test]
fn test_sharded_mappings_new_is_empty() {
    let mappings = ShardedMappings::new();
    assert!(mappings.is_empty());
    assert_eq!(mappings.len(), 0);
}

#[test]
fn test_sharded_mappings_register_returns_index() {
    let mappings = ShardedMappings::new();
    let idx = mappings.register(42);
    assert_eq!(idx, Some(0));
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_sharded_mappings_register_increments_index() {
    let mappings = ShardedMappings::new();
    assert_eq!(mappings.register(1), Some(0));
    assert_eq!(mappings.register(2), Some(1));
    assert_eq!(mappings.register(3), Some(2));
    assert_eq!(mappings.len(), 3);
}

#[test]
fn test_sharded_mappings_register_duplicate_returns_none() {
    let mappings = ShardedMappings::new();
    mappings.register(42);
    let result = mappings.register(42);
    assert_eq!(result, None);
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_sharded_mappings_get_idx() {
    let mappings = ShardedMappings::new();
    mappings.register(42);
    assert_eq!(mappings.get_idx(42), Some(0));
    assert_eq!(mappings.get_idx(999), None);
}

#[test]
fn test_sharded_mappings_get_id() {
    let mappings = ShardedMappings::new();
    mappings.register(42);
    assert_eq!(mappings.get_id(0), Some(42));
    assert_eq!(mappings.get_id(999), None);
}

#[test]
fn test_sharded_mappings_remove() {
    let mappings = ShardedMappings::new();
    mappings.register(42);
    let result = mappings.remove(42);
    assert_eq!(result, Some(0));
    assert!(mappings.is_empty());
    assert_eq!(mappings.get_idx(42), None);
    assert_eq!(mappings.get_id(0), None);
}

#[test]
fn test_sharded_mappings_remove_nonexistent() {
    let mappings = ShardedMappings::new();
    assert_eq!(mappings.remove(999), None);
}

#[test]
fn test_sharded_mappings_contains() {
    let mappings = ShardedMappings::new();
    mappings.register(42);
    assert!(mappings.contains(42));
    assert!(!mappings.contains(999));
}

#[test]
fn test_sharded_mappings_with_capacity() {
    let mappings = ShardedMappings::with_capacity(1000);
    assert!(mappings.is_empty());
    assert_eq!(mappings.register(1), Some(0));
}

#[test]
fn test_sharded_mappings_register_batch() {
    let mappings = ShardedMappings::new();
    let ids = vec![10, 20, 30, 40, 50];
    let results = mappings.register_batch(&ids);
    assert_eq!(results.len(), 5);
    assert_eq!(mappings.len(), 5);
    for (id, idx) in results {
        assert_eq!(mappings.get_idx(id), Some(idx));
    }
}

#[test]
fn test_sharded_mappings_register_batch_with_duplicates() {
    let mappings = ShardedMappings::new();
    mappings.register(20); // Pre-register one ID
    let ids = vec![10, 20, 30]; // 20 is duplicate
    let results = mappings.register_batch(&ids);
    assert_eq!(results.len(), 2);
    assert_eq!(mappings.len(), 3);
}

#[test]
fn test_sharded_mappings_iter() {
    let mappings = ShardedMappings::new();
    mappings.register(10);
    mappings.register(20);
    mappings.register(30);
    let items: Vec<(u64, usize)> = mappings.iter().collect();
    assert_eq!(items.len(), 3);
}

// -------------------------------------------------------------------------
// Concurrency tests - Critical for EPIC-A validation
// -------------------------------------------------------------------------

#[test]
fn test_sharded_mappings_concurrent_register() {
    let mappings = Arc::new(ShardedMappings::new());
    let num_threads = 8;
    let ids_per_thread = 1000;

    let handles: Vec<_> = (0..num_threads)
        .map(|t| {
            let m = Arc::clone(&mappings);
            thread::spawn(move || {
                let start = t * ids_per_thread;
                let end = start + ids_per_thread;
                let mut registered = 0;
                for id in start..end {
                    if m.register(id as u64).is_some() {
                        registered += 1;
                    }
                }
                registered
            })
        })
        .collect();

    let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
    assert_eq!(total, num_threads * ids_per_thread);
    assert_eq!(mappings.len(), num_threads * ids_per_thread);
}

#[test]
fn test_sharded_mappings_concurrent_register_same_ids() {
    let mappings = Arc::new(ShardedMappings::new());
    let num_threads = 16;
    let num_ids = 100;

    let handles: Vec<_> = (0..num_threads)
        .map(|_| {
            let m = Arc::clone(&mappings);
            thread::spawn(move || {
                let mut registered = 0;
                for id in 0..num_ids {
                    if m.register(id as u64).is_some() {
                        registered += 1;
                    }
                }
                registered
            })
        })
        .collect();

    let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
    assert_eq!(total, num_ids);
    assert_eq!(mappings.len(), num_ids);
}

#[test]
fn test_sharded_mappings_concurrent_read_write() {
    let mappings = Arc::new(ShardedMappings::new());

    for i in 0..1000 {
        mappings.register(i);
    }

    let num_readers = 4;
    let num_writers = 4;
    let mut handles = vec![];

    for _ in 0..num_readers {
        let m = Arc::clone(&mappings);
        handles.push(thread::spawn(move || {
            for _ in 0..10000 {
                let _ = m.get_idx(500);
                let _ = m.get_id(500);
                let _ = m.contains(500);
            }
        }));
    }

    for t in 0..num_writers {
        let m = Arc::clone(&mappings);
        handles.push(thread::spawn(move || {
            let start = 1000 + t * 100;
            for i in start..(start + 100) {
                m.register(i as u64);
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread should not panic");
    }

    assert_eq!(mappings.len(), 1000 + num_writers * 100);
}

#[test]
fn test_sharded_mappings_no_data_race() {
    let mappings = Arc::new(ShardedMappings::new());
    let num_threads = 8;
    let ops_per_thread = 1000;

    let handles: Vec<_> = (0..num_threads)
        .map(|t| {
            let m = Arc::clone(&mappings);
            thread::spawn(move || {
                for i in 0..ops_per_thread {
                    #[allow(clippy::cast_sign_loss)]
                    let id = (t * ops_per_thread + i) as u64;

                    if let Some(idx) = m.register(id) {
                        assert_eq!(m.get_idx(id), Some(idx));
                        assert_eq!(m.get_id(idx), Some(id));
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("No data race");
    }

    for entry in mappings.iter() {
        let (id, idx) = entry;
        assert_eq!(mappings.get_idx(id), Some(idx));
        assert_eq!(mappings.get_id(idx), Some(id));
    }
}

// -------------------------------------------------------------------------
// Serialization tests (TDD for HnswIndex migration)
// -------------------------------------------------------------------------

#[test]
fn test_sharded_mappings_as_parts_empty() {
    let mappings = ShardedMappings::new();
    let (id_to_idx, idx_to_id, next_idx) = mappings.as_parts();
    assert!(id_to_idx.is_empty());
    assert!(idx_to_id.is_empty());
    assert_eq!(next_idx, 0);
}

#[test]
fn test_sharded_mappings_as_parts_with_data() {
    let mappings = ShardedMappings::new();
    mappings.register(100);
    mappings.register(200);
    mappings.register(300);

    let (id_to_idx, idx_to_id, next_idx) = mappings.as_parts();
    assert_eq!(id_to_idx.len(), 3);
    assert_eq!(idx_to_id.len(), 3);
    assert_eq!(next_idx, 3);
    assert_eq!(id_to_idx.get(&100), Some(&0));
    assert_eq!(id_to_idx.get(&200), Some(&1));
    assert_eq!(id_to_idx.get(&300), Some(&2));
}

#[test]
fn test_sharded_mappings_from_parts_roundtrip() {
    let original = ShardedMappings::new();
    original.register(42);
    original.register(100);
    original.register(999);

    let (id_to_idx, idx_to_id, next_idx) = original.as_parts();
    let restored = ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx);

    assert_eq!(restored.len(), 3);
    assert_eq!(restored.get_idx(42), Some(0));
    assert_eq!(restored.get_idx(100), Some(1));
    assert_eq!(restored.get_idx(999), Some(2));
    assert_eq!(restored.get_id(0), Some(42));
    assert_eq!(restored.get_id(1), Some(100));
    assert_eq!(restored.get_id(2), Some(999));
}

#[test]
fn test_sharded_mappings_from_parts_preserves_next_idx() {
    let original = ShardedMappings::new();
    original.register(1);
    original.register(2);

    let (id_to_idx, idx_to_id, next_idx) = original.as_parts();
    let restored = ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx);

    let new_idx = restored.register(3);
    assert_eq!(new_idx, Some(2));
}
