#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for memory pool and concurrent memory pool.

use super::concurrent::ConcurrentMemoryPool;
use super::*;

#[test]
fn test_memory_pool_allocate_store_get() {
    let mut pool: MemoryPool<u64> = MemoryPool::new(4);

    let idx1 = pool.allocate();
    pool.store(idx1, 42);

    let idx2 = pool.allocate();
    pool.store(idx2, 100);

    assert_eq!(pool.get(idx1), Some(&42));
    assert_eq!(pool.get(idx2), Some(&100));
    assert_eq!(pool.allocated_count(), 2);
}

#[test]
fn test_memory_pool_deallocate_reuse() {
    let mut pool: MemoryPool<u64> = MemoryPool::new(4);

    let idx1 = pool.allocate();
    pool.store(idx1, 42);
    pool.deallocate(idx1);

    // Next allocation should reuse the freed slot
    let idx2 = pool.allocate();
    assert_eq!(idx1.as_usize(), idx2.as_usize());
}

#[test]
fn test_memory_pool_grow() {
    let mut pool: MemoryPool<u64> = MemoryPool::new(2);

    // Allocate more than chunk size
    for i in 0..10 {
        let idx = pool.allocate();
        pool.store(idx, i);
    }

    assert_eq!(pool.allocated_count(), 10);
    assert!(pool.capacity() >= 10);
}

#[test]
fn test_concurrent_pool_basic() {
    let pool: ConcurrentMemoryPool<u64> = ConcurrentMemoryPool::new(2, 4);

    let h1 = pool.allocate();
    pool.store(h1, 42);

    let h2 = pool.allocate();
    pool.store(h2, 100);

    assert_eq!(pool.with_value(h1, |v| *v), Some(42));
    assert_eq!(pool.with_value(h2, |v| *v), Some(100));
}

#[test]
fn test_concurrent_pool_multithread() {
    use std::sync::Arc;
    use std::thread;

    let pool = Arc::new(ConcurrentMemoryPool::<u64>::new(4, 16));
    let mut handles = Vec::new();

    for t in 0..4 {
        let pool_clone = Arc::clone(&pool);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                let h = pool_clone.allocate();
                pool_clone.store(h, (t * 100 + i) as u64);
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(pool.allocated_count(), 400);
}

#[test]
fn test_memory_pool_allocate_batch() {
    let mut pool: MemoryPool<u64> = MemoryPool::new(4);

    // Allocate a batch of 10 slots
    let indices = pool.allocate_batch(10);
    assert_eq!(indices.len(), 10);

    // Store values
    for (i, idx) in indices.iter().enumerate() {
        pool.store(*idx, i as u64 * 10);
    }

    // Verify all values
    for (i, idx) in indices.iter().enumerate() {
        assert_eq!(pool.get(*idx), Some(&(i as u64 * 10)));
    }

    assert_eq!(pool.allocated_count(), 10);
}

#[test]
fn test_memory_pool_allocate_batch_empty() {
    let mut pool: MemoryPool<u64> = MemoryPool::new(4);
    let indices = pool.allocate_batch(0);
    assert!(indices.is_empty());
    assert_eq!(pool.allocated_count(), 0);
}

#[test]
fn test_memory_pool_allocate_batch_with_free_slots() {
    let mut pool: MemoryPool<u64> = MemoryPool::new(4);

    // Allocate and deallocate some slots
    let idx1 = pool.allocate();
    let idx2 = pool.allocate();
    pool.store(idx1, 1);
    pool.store(idx2, 2);
    pool.deallocate(idx1);
    pool.deallocate(idx2);

    // Now batch allocate - should reuse freed slots first
    let indices = pool.allocate_batch(5);
    assert_eq!(indices.len(), 5);
    assert_eq!(pool.allocated_count(), 5);
}

#[test]
fn test_memory_pool_prefetch() {
    let mut pool: MemoryPool<u64> = MemoryPool::new(4);

    let idx = pool.allocate();
    pool.store(idx, 42);

    // Prefetch should not crash
    pool.prefetch(idx);

    // Value should still be accessible
    assert_eq!(pool.get(idx), Some(&42));
}

/// Regression test for BUG-1: allocate() without store() must not cause UB in Drop.
/// Previously, Drop assumed all non-free slots were initialized, which was wrong.
#[test]
fn test_allocate_without_store_no_ub() {
    let mut pool: MemoryPool<String> = MemoryPool::new(4);

    // Allocate slots but DON'T call store() - this used to cause UB in Drop
    let _idx1 = pool.allocate();
    let _idx2 = pool.allocate();
    let _idx3 = pool.allocate();

    // Only store to one slot
    let idx4 = pool.allocate();
    pool.store(idx4, "initialized".to_string());

    // Drop should only drop idx4, not the uninitialized slots
    // If this test doesn't crash/ASAN error, the fix works
    drop(pool);
}

/// Regression test: deallocate() on uninitialized slot must not crash
#[test]
fn test_deallocate_uninitialized_slot() {
    let mut pool: MemoryPool<String> = MemoryPool::new(4);

    let idx = pool.allocate();
    // Don't call store()
    pool.deallocate(idx); // Should not crash

    // Pool should still be usable
    let idx2 = pool.allocate();
    pool.store(idx2, "test".to_string());
    assert_eq!(pool.get(idx2), Some(&"test".to_string()));
}
