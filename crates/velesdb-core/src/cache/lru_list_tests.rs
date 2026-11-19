//! Tests for `lru_list`: the single-threaded recency list behind `LruCache`.
//!
//! The property test drives `LruList` and a deliberately naive reference
//! model (a `Vec` ordered LRU to MRU) with the same random operations, and
//! requires identical answers and an identical recency order after each one.

use super::lru_list::LruList;
use proptest::prelude::*;

#[test]
fn test_get_moves_the_entry_to_mru() {
    let mut list = LruList::new(3);
    list.insert(1, "one");
    list.insert(2, "two");
    list.insert(3, "three");

    assert_eq!(list.get(&1), Some(&"one"));

    assert_eq!(list.keys_lru_to_mru(), vec![2, 3, 1]);
}

#[test]
fn test_insert_at_capacity_evicts_and_returns_the_lru_entry() {
    let mut list = LruList::new(2);
    list.insert(1, "one");
    list.insert(2, "two");

    assert_eq!(list.insert(3, "three"), Some((1, "one")));

    assert_eq!(list.keys_lru_to_mru(), vec![2, 3]);
}

#[test]
fn test_insert_of_an_existing_key_replaces_its_value_and_evicts_nothing() {
    let mut list = LruList::new(2);
    list.insert(1, "one");
    list.insert(2, "two");

    assert_eq!(list.insert(1, "uno"), None);

    assert_eq!(list.peek(&1), Some(&"uno"));
    assert_eq!(list.keys_lru_to_mru(), vec![2, 1]);
}

#[test]
fn test_remove_keeps_the_order_of_the_others_and_frees_the_slot() {
    let mut list = LruList::new(3);
    list.insert(1, "one");
    list.insert(2, "two");
    list.insert(3, "three");

    assert_eq!(list.remove(&2), Some("two"));
    assert_eq!(list.keys_lru_to_mru(), vec![1, 3]);

    // The freed slot is reused: a fourth key fits without evicting.
    assert_eq!(list.insert(4, "four"), None);
    assert_eq!(list.keys_lru_to_mru(), vec![1, 3, 4]);
}

#[test]
fn test_peek_does_not_change_the_order() {
    let mut list = LruList::new(2);
    list.insert(1, "one");
    list.insert(2, "two");

    assert_eq!(list.peek(&1), Some(&"one"));

    assert_eq!(list.keys_lru_to_mru(), vec![1, 2]);
}

#[test]
fn test_zero_capacity_stores_nothing() {
    let mut list = LruList::new(0);

    assert_eq!(list.insert(1, "one"), Some((1, "one")));

    assert!(list.is_empty());
}

#[test]
fn test_clear_empties_the_list_and_keeps_it_usable() {
    let mut list = LruList::new(2);
    list.insert(1, "one");
    list.insert(2, "two");

    list.clear();
    assert!(list.is_empty());

    list.insert(3, "three");
    assert_eq!(list.keys_lru_to_mru(), vec![3]);
}

/// One operation of the property test. Keys come from a small range so
/// that hits, updates, removals of present keys and evictions all occur.
#[derive(Debug, Clone)]
enum Op {
    Insert(u8, u16),
    Get(u8),
    Peek(u8),
    Remove(u8),
    Clear,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..12_u8, any::<u16>()).prop_map(|(k, v)| Op::Insert(k, v)),
        3 => (0..12_u8).prop_map(Op::Get),
        1 => (0..12_u8).prop_map(Op::Peek),
        2 => (0..12_u8).prop_map(Op::Remove),
        1 => Just(Op::Clear),
    ]
}

/// The reference: entries ordered LRU to MRU, every operation a linear
/// scan, so its correctness is evident by reading it.
struct Model {
    capacity: usize,
    entries: Vec<(u8, u16)>,
}

impl Model {
    fn position(&self, key: u8) -> Option<usize> {
        self.entries.iter().position(|&(k, _)| k == key)
    }

    fn insert(&mut self, key: u8, value: u16) -> Option<(u8, u16)> {
        if self.capacity == 0 {
            return Some((key, value));
        }
        if let Some(i) = self.position(key) {
            self.entries.remove(i);
            self.entries.push((key, value));
            return None;
        }
        let evicted = (self.entries.len() >= self.capacity).then(|| self.entries.remove(0));
        self.entries.push((key, value));
        evicted
    }

    fn get(&mut self, key: u8) -> Option<u16> {
        let entry = self.entries.remove(self.position(key)?);
        self.entries.push(entry);
        Some(entry.1)
    }

    fn peek(&self, key: u8) -> Option<u16> {
        self.position(key).map(|i| self.entries[i].1)
    }

    fn remove(&mut self, key: u8) -> Option<u16> {
        self.position(key).map(|i| self.entries.remove(i).1)
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn prop_lru_list_matches_the_reference_model(
        capacity in 0..6_usize,
        ops in prop::collection::vec(op(), 0..200),
    ) {
        let mut list = LruList::new(capacity);
        let mut model = Model { capacity, entries: Vec::new() };

        for op in ops {
            match op {
                Op::Insert(k, v) => prop_assert_eq!(list.insert(k, v), model.insert(k, v)),
                Op::Get(k) => prop_assert_eq!(list.get(&k).copied(), model.get(k)),
                Op::Peek(k) => prop_assert_eq!(list.peek(&k).copied(), model.peek(k)),
                Op::Remove(k) => prop_assert_eq!(list.remove(&k), model.remove(k)),
                Op::Clear => {
                    list.clear();
                    model.entries.clear();
                }
            }
            let expected: Vec<u8> = model.entries.iter().map(|&(k, _)| k).collect();
            prop_assert_eq!(list.keys_lru_to_mru(), expected);
            prop_assert_eq!(list.len(), model.entries.len());
        }
    }
}
