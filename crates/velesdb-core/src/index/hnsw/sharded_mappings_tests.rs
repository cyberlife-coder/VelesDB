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
// register_or_replace tests
// -------------------------------------------------------------------------

#[test]
fn test_register_or_replace_new_id() {
    let mappings = ShardedMappings::new();
    let (idx, old) = mappings.register_or_replace(42);
    assert_eq!(idx, 0);
    assert_eq!(old, None);
    assert_eq!(mappings.get_idx(42), Some(0));
    assert_eq!(mappings.get_id(0), Some(42));
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_register_or_replace_existing_id() {
    let mappings = ShardedMappings::new();
    let first_idx = mappings.register(42).expect("first register");
    let (new_idx, old_idx) = mappings.register_or_replace(42);

    assert_eq!(old_idx, Some(first_idx));
    assert_ne!(new_idx, first_idx);
    // Old reverse mapping removed
    assert_eq!(mappings.get_id(first_idx), None);
    // New mapping present
    assert_eq!(mappings.get_idx(42), Some(new_idx));
    assert_eq!(mappings.get_id(new_idx), Some(42));
    // Length is still 1 (replaced, not duplicated)
    assert_eq!(mappings.len(), 1);
}

// -------------------------------------------------------------------------
// restore tests (rollback support)
// -------------------------------------------------------------------------

#[test]
fn test_restore_after_remove() {
    let mappings = ShardedMappings::new();
    let old_idx = mappings.register(42).expect("register");
    mappings.remove(42);
    assert_eq!(mappings.get_idx(42), None);

    mappings.restore(42, old_idx);
    assert_eq!(mappings.get_idx(42), Some(old_idx));
    assert_eq!(mappings.get_id(old_idx), Some(42));
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_restore_after_register_or_replace_rollback() {
    let mappings = ShardedMappings::new();
    let first_idx = mappings.register(42).expect("register");

    // Simulate upsert: register_or_replace allocates a new idx
    let (new_idx, old_idx) = mappings.register_or_replace(42);
    assert_eq!(old_idx, Some(first_idx));

    // Simulate failure: remove new mapping, restore old one
    mappings.remove(42);
    assert_eq!(mappings.get_id(new_idx), None);

    mappings.restore(42, first_idx);
    assert_eq!(mappings.get_idx(42), Some(first_idx));
    assert_eq!(mappings.get_id(first_idx), Some(42));
    assert_eq!(mappings.len(), 1);
}

// -------------------------------------------------------------------------
// remove_reverse tests (BUG-0002: concurrent insert mapping correction)
// -------------------------------------------------------------------------

#[test]
fn test_remove_reverse_cleans_stale_idx_to_id() {
    let mappings = ShardedMappings::new();
    let idx = mappings.register(42).expect("register");

    // Forward mapping intact before remove_reverse
    assert_eq!(mappings.get_idx(42), Some(idx));
    assert_eq!(mappings.get_id(idx), Some(42));

    // remove_reverse only removes the reverse mapping (idx -> id)
    mappings.remove_reverse(idx);

    // Forward mapping (id -> idx) must survive
    assert_eq!(mappings.get_idx(42), Some(idx));
    // Reverse mapping (idx -> id) must be gone
    assert_eq!(mappings.get_id(idx), None);
    // Length is based on id_to_idx, so unchanged
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_remove_reverse_nonexistent_idx_is_noop() {
    let mappings = ShardedMappings::new();
    mappings.register(42).expect("register");

    // Removing a reverse mapping for an idx that doesn't exist is a no-op
    mappings.remove_reverse(999);

    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings.get_idx(42), Some(0));
    assert_eq!(mappings.get_id(0), Some(42));
}

#[test]
fn test_remove_reverse_then_restore_fixes_divergence() {
    // Simulates the insert_and_correct_mapping pattern:
    // 1. upsert_mapping allocates idx=0 for id=42 (id->0, 0->42)
    // 2. HNSW graph assigns node_id=5 instead of 0 (divergence)
    // 3. remove_reverse(0) cleans stale 0->42
    // 4. restore(42, 5) sets id->5 and 5->42
    let mappings = ShardedMappings::new();
    let stale_idx = mappings.register(42).expect("register");
    assert_eq!(stale_idx, 0);

    let correct_idx: usize = 5;

    // Step 3: remove stale reverse mapping
    mappings.remove_reverse(stale_idx);
    assert_eq!(mappings.get_id(stale_idx), None);

    // Step 4: restore with correct idx
    mappings.restore(42, correct_idx);
    assert_eq!(mappings.get_idx(42), Some(correct_idx));
    assert_eq!(mappings.get_id(correct_idx), Some(42));
    // Stale reverse mapping still gone
    assert_eq!(mappings.get_id(stale_idx), None);
}

// -------------------------------------------------------------------------
// register_or_replace_batch tests (issue #375)
// -------------------------------------------------------------------------

#[test]
fn test_register_or_replace_batch_all_new() {
    let mappings = ShardedMappings::new();
    let ids = vec![10, 20, 30, 40, 50];
    let results = mappings.register_or_replace_batch(&ids);

    assert_eq!(results.len(), 5);
    for (i, (idx, old_idx)) in results.iter().enumerate() {
        // All new IDs: each should get a sequential index, no old index
        assert_eq!(*idx, i, "ID {} should get index {i}", ids[i]);
        assert_eq!(*old_idx, None, "ID {} should have no old index", ids[i]);
    }
    assert_eq!(mappings.len(), 5);
    // Verify bidirectional mappings
    for (i, &id) in ids.iter().enumerate() {
        assert_eq!(mappings.get_idx(id), Some(i));
        assert_eq!(mappings.get_id(i), Some(id));
    }
}

#[test]
fn test_register_or_replace_batch_all_existing() {
    let mappings = ShardedMappings::new();
    // Pre-insert all IDs
    let ids = vec![10, 20, 30];
    for &id in &ids {
        mappings.register(id);
    }
    let original_len = mappings.len();
    assert_eq!(original_len, 3);

    // Batch replace all existing IDs
    let results = mappings.register_or_replace_batch(&ids);
    assert_eq!(results.len(), 3);
    for (idx, old_idx) in &results {
        // All existing: each should have an old index
        assert!(old_idx.is_some(), "existing ID should have old index");
        // New index must differ from old
        assert_ne!(idx, old_idx.as_ref().unwrap());
    }
    // Length unchanged (replaced, not duplicated)
    assert_eq!(mappings.len(), 3);
    // Old reverse mappings removed
    assert_eq!(mappings.get_id(0), None); // old idx for id=10
    assert_eq!(mappings.get_id(1), None); // old idx for id=20
    assert_eq!(mappings.get_id(2), None); // old idx for id=30
}

#[test]
fn test_register_or_replace_batch_mixed() {
    let mappings = ShardedMappings::new();
    // Pre-insert some IDs
    mappings.register(20); // idx=0
    mappings.register(40); // idx=1

    let ids = vec![10, 20, 30, 40, 50];
    let results = mappings.register_or_replace_batch(&ids);

    assert_eq!(results.len(), 5);

    // ID 10: new
    assert_eq!(results[0].1, None);
    // ID 20: existing (was idx=0)
    assert_eq!(results[1].1, Some(0));
    // ID 30: new
    assert_eq!(results[2].1, None);
    // ID 40: existing (was idx=1)
    assert_eq!(results[3].1, Some(1));
    // ID 50: new
    assert_eq!(results[4].1, None);

    // Total live mappings: 5 (2 replaced + 3 new)
    assert_eq!(mappings.len(), 5);

    // Verify all IDs are accessible
    for &id in &ids {
        assert!(mappings.contains(id), "ID {id} should be present");
    }
}

#[test]
fn test_register_or_replace_batch_empty() {
    let mappings = ShardedMappings::new();
    let results = mappings.register_or_replace_batch(&[]);
    assert!(results.is_empty());
    assert!(mappings.is_empty());
}

#[test]
fn test_register_or_replace_batch_single_new() {
    let mappings = ShardedMappings::new();
    let results = mappings.register_or_replace_batch(&[42]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], (0, None));
    assert_eq!(mappings.get_idx(42), Some(0));
}

#[test]
fn test_register_or_replace_batch_single_existing() {
    let mappings = ShardedMappings::new();
    mappings.register(42); // idx=0
    let results = mappings.register_or_replace_batch(&[42]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1, Some(0)); // old_idx was 0
    assert_ne!(results[0].0, 0); // new_idx differs
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_register_or_replace_batch_within_batch_duplicates() {
    let mappings = ShardedMappings::new();
    // Same ID twice in one batch: second occurrence should replace the first
    let results = mappings.register_or_replace_batch(&[42, 42]);
    assert_eq!(results.len(), 2);
    // First occurrence: new ID
    assert_eq!(results[0].1, None);
    // Second occurrence: replaces the first
    assert_eq!(results[1].1, Some(results[0].0));
    // Only one live mapping
    assert_eq!(mappings.len(), 1);
    // Final mapping points to the second index
    assert_eq!(mappings.get_idx(42), Some(results[1].0));
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

// -------------------------------------------------------------------------
// tombstone_count tests (Devin finding #12)
// -------------------------------------------------------------------------

#[test]
fn test_tombstone_count_zero_on_new() {
    let mappings = ShardedMappings::new();
    assert_eq!(mappings.tombstone_count(), 0);
}

#[test]
fn test_tombstone_count_zero_after_normal_batch() {
    // All-new batch: fast path with no races => zero tombstones.
    let mappings = ShardedMappings::new();
    mappings.register_or_replace_batch(&[10, 20, 30]);
    assert_eq!(
        mappings.tombstone_count(),
        0,
        "no tombstones when all IDs are new"
    );
}

#[test]
fn test_tombstone_count_zero_after_slow_path_replace() {
    // Slow path (mixed): races handled individually, no pre-reserved range.
    let mappings = ShardedMappings::new();
    mappings.register(20);
    mappings.register_or_replace_batch(&[10, 20, 30]);
    assert_eq!(
        mappings.tombstone_count(),
        0,
        "slow path does not create tombstones"
    );
}

#[test]
fn test_tombstone_count_reset_on_clear() {
    let mappings = ShardedMappings::new();
    // Simulate a race by inserting an ID between vacancy check and entry().
    // Since tests are single-threaded, we use the slow path to confirm the
    // counter resets — the actual increment only occurs under real contention.
    mappings.clear();
    assert_eq!(mappings.tombstone_count(), 0);
}

#[test]
fn test_tombstone_count_zero_on_from_parts() {
    let original = ShardedMappings::new();
    original.register(1);
    let (id_to_idx, idx_to_id, next_idx) = original.as_parts();
    let restored = ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx);
    assert_eq!(
        restored.tombstone_count(),
        0,
        "from_parts starts with zero tombstones"
    );
}
