//! Tests for `sharded_mappings` module

use super::native_inner::Placed;
use super::sharded_mappings::*;
use std::sync::Arc;
use std::thread;

fn slot(id: u64) -> usize {
    usize::try_from(id).expect("test: slot fits usize")
}

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
fn test_sharded_mappings_get_idx() {
    let mappings = ShardedMappings::new();
    mappings.assign(42, Placed::for_test(0));
    assert_eq!(mappings.get_idx(42), Some(0));
    assert_eq!(mappings.get_idx(999), None);
}

#[test]
fn test_sharded_mappings_get_id() {
    let mappings = ShardedMappings::new();
    mappings.assign(42, Placed::for_test(0));
    assert_eq!(mappings.get_id(0), Some(42));
    assert_eq!(mappings.get_id(999), None);
}

#[test]
fn test_sharded_mappings_remove() {
    let mappings = ShardedMappings::new();
    mappings.assign(42, Placed::for_test(0));
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
    mappings.assign(42, Placed::for_test(0));
    assert!(mappings.contains(42));
    assert!(!mappings.contains(999));
}

#[test]
fn test_sharded_mappings_with_capacity() {
    let mappings = ShardedMappings::with_capacity(1000);
    assert!(mappings.is_empty());
    assert_eq!(mappings.assign(1, Placed::for_test(0)), None);
    assert_eq!(mappings.get_idx(1), Some(0));
}

#[test]
fn test_sharded_mappings_iter() {
    let mappings = ShardedMappings::new();
    mappings.assign(10, Placed::for_test(0));
    mappings.assign(20, Placed::for_test(1));
    mappings.assign(30, Placed::for_test(2));
    let mut items: Vec<(u64, usize)> = mappings.iter().collect();
    assert_eq!(items.len(), 3);
    // DashMap iteration order is non-deterministic; sort before exact compare.
    items.sort_by_key(|(id, _)| *id);
    assert_eq!(items, vec![(10, 0), (20, 1), (30, 2)]);
}

// -------------------------------------------------------------------------
// Concurrency tests - Critical for EPIC-A validation
// -------------------------------------------------------------------------

#[test]
fn test_sharded_mappings_concurrent_read_write() {
    let mappings = Arc::new(ShardedMappings::new());

    for id in 0..1000u64 {
        mappings.assign(id, Placed::for_test(slot(id)));
    }

    let num_readers = 4;
    let num_writers = 4u64;
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
            for id in start..(start + 100) {
                m.assign(id, Placed::for_test(slot(id)));
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread should not panic");
    }

    assert_eq!(mappings.len(), 1000 + slot(num_writers) * 100);
}

#[test]
fn test_sharded_mappings_no_data_race() {
    let mappings = Arc::new(ShardedMappings::new());
    let num_threads = 8u64;
    let ops_per_thread = 1000u64;

    let handles: Vec<_> = (0..num_threads)
        .map(|t| {
            let m = Arc::clone(&mappings);
            thread::spawn(move || {
                for i in 0..ops_per_thread {
                    let id = t * ops_per_thread + i;
                    m.assign(id, Placed::for_test(slot(id)));
                    assert_eq!(m.get_idx(id), Some(slot(id)));
                    assert_eq!(m.get_id(slot(id)), Some(id));
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

#[test]
fn concurrent_assigns_of_disjoint_slots_stay_consistent() {
    let mappings = Arc::new(ShardedMappings::new());
    let handles: Vec<_> = (0..8u64)
        .map(|t| {
            let mappings = Arc::clone(&mappings);
            thread::spawn(move || {
                for i in 0..500u64 {
                    let id = t * 1_000 + i;
                    mappings.assign(id, Placed::for_test(slot(id)));
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("test: thread");
    }
    for t in 0..8u64 {
        for i in 0..500u64 {
            let id = t * 1_000 + i;
            assert_eq!(mappings.get_idx(id), Some(slot(id)));
            assert_eq!(mappings.get_id(slot(id)), Some(id));
        }
    }
    assert_eq!(mappings.len(), 4_000);
}

/// Sixteen writers move the same hundred ids, each onto slots of its own.
/// Whatever order they land in, every id must end on one slot that points
/// back to it, and every slot an id moved off must be retired.
#[test]
fn concurrent_assigns_of_one_id_leave_exactly_one_slot_mapped() {
    let mappings = Arc::new(ShardedMappings::new());
    let (threads, ids) = (16u64, 100u64);
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let m = Arc::clone(&mappings);
            thread::spawn(move || {
                for id in 0..ids {
                    m.assign(id, Placed::for_test(slot(t * ids + id)));
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("test: thread");
    }

    assert_eq!(mappings.len(), slot(ids));
    for id in 0..ids {
        let held = mappings.get_idx(id).expect("every id stays mapped");
        assert_eq!(
            mappings.get_id(held),
            Some(id),
            "id {id}'s slot must point back to it"
        );
    }
    let (_, reverse, _) = mappings.as_parts();
    assert_eq!(
        reverse.len(),
        slot(ids),
        "a slot an id moved off still answers for it"
    );
}

// -------------------------------------------------------------------------
// remove_reverse tests
// -------------------------------------------------------------------------

#[test]
fn test_remove_reverse_cleans_stale_idx_to_id() {
    let mappings = ShardedMappings::new();
    mappings.assign(42, Placed::for_test(0));

    // remove_reverse only removes the reverse mapping (idx -> id)
    mappings.remove_reverse(0, 42);

    // Forward mapping (id -> idx) must survive
    assert_eq!(mappings.get_idx(42), Some(0));
    // Reverse mapping (idx -> id) must be gone
    assert_eq!(mappings.get_id(0), None);
    // Length is based on id_to_idx, so unchanged
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_remove_reverse_nonexistent_idx_is_noop() {
    let mappings = ShardedMappings::new();
    mappings.assign(42, Placed::for_test(0));

    // Removing a reverse mapping for an idx that doesn't exist is a no-op
    mappings.remove_reverse(999, 42);

    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings.get_idx(42), Some(0));
    assert_eq!(mappings.get_id(0), Some(42));
}

#[test]
fn test_remove_reverse_leaves_another_owners_entry() {
    let mappings = ShardedMappings::new();
    mappings.assign(84, Placed::for_test(5));

    mappings.remove_reverse(5, 42);

    assert_eq!(mappings.get_id(5), Some(84));
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
    mappings.assign(100, Placed::for_test(0));
    mappings.assign(200, Placed::for_test(1));
    mappings.assign(300, Placed::for_test(2));

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
    original.assign(42, Placed::for_test(0));
    original.assign(100, Placed::for_test(1));
    original.assign(999, Placed::for_test(2));

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
    original.assign(1, Placed::for_test(0));
    original.assign(2, Placed::for_test(1));

    let (id_to_idx, idx_to_id, next_idx) = original.as_parts();
    let restored = ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx);

    assert_eq!(restored.next_idx(), 2);
}

#[test]
fn test_clear_resets_mappings_and_next_idx() {
    let mappings = ShardedMappings::new();
    mappings.assign(10, Placed::for_test(0));
    mappings.assign(20, Placed::for_test(1));
    mappings.assign(30, Placed::for_test(2));
    assert_eq!(mappings.next_idx(), 3, "next_idx advanced before clear");

    mappings.clear();

    assert!(mappings.is_empty(), "clear empties id_to_idx/idx_to_id");
    assert!(!mappings.contains(10));
    assert_eq!(mappings.get_id(0), None);
    assert_eq!(mappings.next_idx(), 0, "clear resets next_idx");
}

// -------------------------------------------------------------------------
// assign: the mapping follows the slot the vector was given (#2246)
// -------------------------------------------------------------------------

#[test]
fn assign_maps_both_directions() {
    let mappings = ShardedMappings::new();
    assert_eq!(mappings.assign(42, Placed::for_test(7)), None);
    assert_eq!(mappings.get_idx(42), Some(7));
    assert_eq!(mappings.get_id(7), Some(42));
}

#[test]
fn assign_to_a_new_slot_retires_the_old_reverse_entry() {
    let mappings = ShardedMappings::new();
    mappings.assign(42, Placed::for_test(3));
    assert_eq!(mappings.assign(42, Placed::for_test(9)), Some(3));
    assert_eq!(mappings.get_idx(42), Some(9));
    assert_eq!(mappings.get_id(9), Some(42));
    assert_eq!(
        mappings.get_id(3),
        None,
        "the old slot must become a tombstone"
    );
    assert_eq!(mappings.len(), 1);
}

#[test]
fn assign_to_the_same_slot_changes_nothing() {
    let mappings = ShardedMappings::new();
    mappings.assign(42, Placed::for_test(5));
    assert_eq!(mappings.assign(42, Placed::for_test(5)), None);
    assert_eq!(mappings.get_idx(42), Some(5));
    assert_eq!(mappings.get_id(5), Some(42));
}

#[test]
fn assign_keeps_next_idx_above_every_slot_in_use() {
    let mappings = ShardedMappings::new();
    mappings.assign(1, Placed::for_test(10));
    assert_eq!(mappings.next_idx(), 11);
    mappings.assign(2, Placed::for_test(4));
    assert_eq!(mappings.next_idx(), 11, "next_idx never goes down");
}

/// The arena hands every slot out once, so a slot another id holds is a
/// broken caller, not a move: debug builds refuse it rather than let two ids
/// share one vector (#2246).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "already belongs to id")]
fn assign_refuses_a_slot_another_id_holds() {
    let mappings = ShardedMappings::new();
    mappings.assign(1, Placed::for_test(7));
    mappings.assign(2, Placed::for_test(7));
}

/// The refusal comes before either map changes: once the panic is caught, the
/// slot still names its owner and the refused id maps nowhere.
#[test]
#[cfg(debug_assertions)]
fn a_refused_slot_leaves_both_maps_untouched() {
    let mappings = ShardedMappings::new();
    mappings.assign(1, Placed::for_test(7));
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        mappings.assign(2, Placed::for_test(7))
    }));
    assert!(refused.is_err(), "a slot another id holds must be refused");
    assert_eq!(mappings.get_id(7), Some(1));
    assert_eq!(mappings.get_idx(2), None);
    assert_eq!(mappings.get_idx(1), Some(7));
}
