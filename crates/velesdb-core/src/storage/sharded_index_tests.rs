#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for `sharded_index` module - Lock-striped sharded index.

use super::sharded_index::*;
use rustc_hash::FxHashMap;

#[test]
fn test_sharded_index_insert_get() {
    let index = ShardedIndex::new();
    index.insert(1, 100);
    index.insert(2, 200);
    index.insert(17, 1700);

    assert_eq!(index.get(1), Some(100));
    assert_eq!(index.get(2), Some(200));
    assert_eq!(index.get(17), Some(1700));
    assert_eq!(index.get(99), None);
}

#[test]
fn test_sharded_index_remove() {
    let index = ShardedIndex::new();
    index.insert(1, 100);
    assert_eq!(index.remove(1), Some(100));
    assert_eq!(index.get(1), None);
    assert_eq!(index.remove(1), None);
}

#[test]
fn test_sharded_index_len() {
    let index = ShardedIndex::new();
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);

    for i in 0..100u64 {
        index.insert(i, i as usize * 10);
    }
    assert_eq!(index.len(), 100);
    assert!(!index.is_empty());
}

#[test]
fn test_sharded_index_to_hashmap() {
    let index = ShardedIndex::new();
    for i in 0..50u64 {
        index.insert(i, i as usize * 10);
    }

    let map = index.to_hashmap();
    assert_eq!(map.len(), 50);
    assert_eq!(map.get(&25), Some(&250));
}

#[test]
fn test_sharded_index_from_hashmap() {
    let mut map = FxHashMap::default();
    for i in 0..50u64 {
        map.insert(i, i as usize * 10);
    }

    let index = ShardedIndex::from_hashmap(map);
    assert_eq!(index.len(), 50);
    assert_eq!(index.get(25), Some(250));
}

#[test]
fn test_sharded_index_max_offset() {
    let index = ShardedIndex::new();
    assert_eq!(index.max_offset(), None);

    index.insert(1, 100);
    index.insert(2, 500);
    index.insert(3, 200);
    assert_eq!(index.max_offset(), Some(500));
}

#[test]
fn test_sharded_index_concurrent_reads() {
    use std::sync::Arc;
    use std::thread;

    let index = Arc::new(ShardedIndex::new());
    for i in 0..1000u64 {
        index.insert(i, i as usize * 10);
    }

    let mut handles = vec![];
    for _ in 0..8 {
        let idx = Arc::clone(&index);
        handles.push(thread::spawn(move || {
            for i in 0..1000u64 {
                assert_eq!(idx.get(i), Some(i as usize * 10));
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}
