//! Model-based tests for `LruList`: a pseudo-random operation stream is
//! replayed against a naive bounded `Vec` model (front = LRU) and both must
//! agree, with the list's structural invariants checked after every step.

use super::lru_list::LruList;

/// Naive reference: O(n) everywhere, obviously correct.
struct Model {
    capacity: usize,
    entries: Vec<(u8, u32)>,
}

impl Model {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Vec::new(),
        }
    }

    fn position(&self, key: u8) -> Option<usize> {
        self.entries.iter().position(|&(k, _)| k == key)
    }

    fn get(&mut self, key: u8) -> Option<u32> {
        let entry = self.entries.remove(self.position(key)?);
        self.entries.push(entry);
        Some(entry.1)
    }

    /// Returns whether an entry was evicted to make room.
    fn insert(&mut self, key: u8, value: u32) -> bool {
        if self.remove(key).is_none() && self.entries.len() >= self.capacity {
            if self.entries.is_empty() {
                return false; // capacity 0 holds nothing
            }
            self.entries.remove(0);
            self.entries.push((key, value));
            return true;
        }
        self.entries.push((key, value));
        false
    }

    fn remove(&mut self, key: u8) -> Option<u32> {
        Some(self.entries.remove(self.position(key)?).1)
    }

    fn keys(&self) -> Vec<u8> {
        self.entries.iter().map(|&(k, _)| k).collect()
    }
}

/// Deterministic xorshift stream: reproducible failures, no extra dependency.
struct Ops(u64);

impl Ops {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn replay_against_model(capacity: usize) {
    let mut list: LruList<u8, u32> = LruList::new(capacity);
    let mut model = Model::new(capacity);
    let mut ops = Ops(0x9E37_79B9_7F4A_7C15);

    for step in 0..20_000_u32 {
        let roll = ops.next();
        // 24 keys: hits, overwrites, evictions and slot recycling all occur.
        let key = u8::try_from((roll >> 8) % 24).expect("< 24 fits in u8");
        let at = format!("capacity {capacity}, step {step}");
        match roll % 4 {
            0 => assert_eq!(list.get(&key).copied(), model.get(key), "get, {at}"),
            1 => assert_eq!(list.remove(&key), model.remove(key), "remove, {at}"),
            _ => assert_eq!(
                list.insert(key, step),
                model.insert(key, step),
                "insert, {at}"
            ),
        }
        assert_eq!(list.checked_order(), model.keys(), "order diverged, {at}");
        assert_eq!(
            list.peek(&key).copied(),
            model.position(key).map(|i| model.entries[i].1),
            "value, {at}"
        );
    }
}

#[test]
fn test_lru_list_matches_naive_model_under_random_operations() {
    for capacity in [0, 1, 8, 30] {
        replay_against_model(capacity);
    }
}

#[test]
fn test_lru_list_peek_does_not_promote() {
    let mut list: LruList<u8, u32> = LruList::new(2);
    list.insert(1, 10);
    list.insert(2, 20);

    assert_eq!(list.peek(&1), Some(&10));
    assert_eq!(list.checked_order(), vec![1, 2]);
}

#[test]
fn test_lru_list_full_insert_recycles_the_lru_entry() {
    let mut list: LruList<u8, u32> = LruList::new(2);
    assert!(!list.insert(1, 10));
    assert!(!list.insert(2, 20));

    assert!(list.insert(3, 30), "a new key into a full list evicts");
    assert!(
        !list.insert(3, 31),
        "overwriting a present key never evicts"
    );

    assert_eq!(list.checked_order(), vec![2, 3]);
    assert_eq!(list.peek(&1), None);
    assert_eq!(list.peek(&3), Some(&31));
}

#[test]
fn test_lru_list_clear_empties_and_stays_usable() {
    let mut list: LruList<u8, u32> = LruList::new(2);
    list.insert(1, 10);
    list.insert(2, 20);

    list.clear();
    list.insert(3, 30);

    assert_eq!(list.checked_order(), vec![3]);
    assert_eq!(list.len(), 1);
}
