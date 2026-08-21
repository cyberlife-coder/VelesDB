//! Unit tests for [`SecondaryIndex::ordered_ids`] — the ordered-iteration
//! primitive for index-backed `ORDER BY <field> LIMIT k` top-k (B001 follow-up).

use super::{F64Key, IdSet, JsonValue, SecondaryIndex};
use parking_lot::RwLock;
use std::collections::BTreeMap;

/// Builds an `IdSet` bucket from ids given in any order.
fn set(ids: &[u64]) -> IdSet {
    let mut out = IdSet::default();
    for &id in ids {
        out.insert(id);
    }
    out
}

/// Mixed-type fixture. Key order is Bool < Number < String (the `JsonValue`
/// `Ord`), and the `Number(1.0)` bucket holds IDs out of order on purpose so
/// the within-bucket ascending-ID sort is exercised.
fn fixture() -> SecondaryIndex {
    let mut map: BTreeMap<JsonValue, IdSet> = BTreeMap::new();
    map.insert(JsonValue::Bool(false), set(&[5]));
    map.insert(JsonValue::Bool(true), set(&[3]));
    map.insert(JsonValue::Number(F64Key::from(1.0)), set(&[10, 2]));
    map.insert(JsonValue::Number(F64Key::from(2.0)), set(&[7]));
    map.insert(JsonValue::String("a".to_string()), set(&[8]));
    SecondaryIndex::BTree(RwLock::new(map))
}

#[test]
fn ascending_walks_keys_then_sorts_each_bucket_by_id() {
    // Bool(false)=5, Bool(true)=3, Number(1)={2,10}, Number(2)=7, String("a")=8
    assert_eq!(fixture().ordered_ids(false, 100), vec![5, 3, 2, 10, 7, 8]);
}

#[test]
fn descending_reverses_key_order_bucket_ids_still_ascending() {
    assert_eq!(fixture().ordered_ids(true, 100), vec![8, 7, 2, 10, 3, 5]);
}

#[test]
fn limit_truncates_after_ordering_not_before() {
    assert_eq!(fixture().ordered_ids(false, 3), vec![5, 3, 2]);
    assert_eq!(fixture().ordered_ids(true, 2), vec![8, 7]);
    // A limit landing mid-bucket keeps the bucket's lowest IDs first.
    assert_eq!(fixture().ordered_ids(false, 4), vec![5, 3, 2, 10]);
}

#[test]
fn limit_zero_returns_empty() {
    assert!(fixture().ordered_ids(false, 0).is_empty());
    assert!(fixture().ordered_ids(true, 0).is_empty());
}

#[test]
fn empty_index_returns_empty() {
    let empty = SecondaryIndex::BTree(RwLock::new(BTreeMap::new()));
    assert!(empty.ordered_ids(false, 10).is_empty());
    assert!(empty.ordered_ids(true, 10).is_empty());
}

#[test]
fn single_key_many_ids_emits_all_sorted() {
    let mut map: BTreeMap<JsonValue, IdSet> = BTreeMap::new();
    map.insert(
        JsonValue::Number(F64Key::from(42.0)),
        set(&[9, 1, 4, 1_000_000_000_000]),
    );
    let idx = SecondaryIndex::BTree(RwLock::new(map));
    assert_eq!(
        idx.ordered_ids(false, 100),
        vec![1, 4, 9, 1_000_000_000_000]
    );
    // descending key order is identical here (one key); bucket stays ID-ascending
    assert_eq!(idx.ordered_ids(true, 2), vec![1, 4]);
}
