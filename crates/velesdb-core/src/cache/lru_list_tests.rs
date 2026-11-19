//! Model-based tests for `LruList`: a pseudo-random operation stream is
//! replayed against a naive `Vec` model (front = LRU) and both must agree,
//! with the list's structural invariants checked after every step.

use super::lru_list::LruList;

/// Naive reference: O(n) everywhere, obviously correct.
#[derive(Default)]
struct Model(Vec<(u8, u32)>);

impl Model {
    fn position(&self, key: u8) -> Option<usize> {
        self.0.iter().position(|&(k, _)| k == key)
    }

    fn get(&mut self, key: u8) -> Option<u32> {
        let entry = self.0.remove(self.position(key)?);
        self.0.push(entry);
        Some(entry.1)
    }

    fn insert(&mut self, key: u8, value: u32) {
        self.remove(key);
        self.0.push((key, value));
    }

    fn remove(&mut self, key: u8) -> Option<u32> {
        Some(self.0.remove(self.position(key)?).1)
    }

    fn pop_lru(&mut self) -> Option<u32> {
        (!self.0.is_empty()).then(|| self.0.remove(0).1)
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

#[test]
fn test_lru_list_matches_naive_model_under_random_operations() {
    let mut list: LruList<u8, u32> = LruList::with_capacity(0);
    let mut model = Model::default();
    let mut ops = Ops(0x9E37_79B9_7F4A_7C15);

    for step in 0..20_000_u32 {
        let roll = ops.next();
        // Small key space forces hits, overwrites and slot reuse.
        let key = u8::try_from((roll >> 8) % 24).expect("< 24 fits in u8");
        match roll % 5 {
            0 => assert_eq!(list.get(&key).copied(), model.get(key), "get, step {step}"),
            1 => assert_eq!(list.remove(&key), model.remove(key), "remove, step {step}"),
            2 => assert_eq!(list.pop_lru(), model.pop_lru(), "pop_lru, step {step}"),
            _ => {
                list.insert(key, step);
                model.insert(key, step);
            }
        }
        let expected: Vec<u8> = model.0.iter().map(|&(k, _)| k).collect();
        assert_eq!(
            list.checked_order(),
            expected,
            "order diverged at step {step}"
        );
    }
}

#[test]
fn test_lru_list_peek_does_not_promote() {
    let mut list: LruList<u8, u32> = LruList::with_capacity(2);
    list.insert(1, 10);
    list.insert(2, 20);

    assert_eq!(list.peek(&1), Some(&10));
    assert_eq!(list.checked_order(), vec![1, 2]);
}

#[test]
fn test_lru_list_clear_empties_and_stays_usable() {
    let mut list: LruList<u8, u32> = LruList::with_capacity(2);
    list.insert(1, 10);
    list.insert(2, 20);

    list.clear();
    list.insert(3, 30);

    assert_eq!(list.checked_order(), vec![3]);
    assert_eq!(list.pop_lru(), Some(30));
    assert_eq!(list.len(), 0);
}
