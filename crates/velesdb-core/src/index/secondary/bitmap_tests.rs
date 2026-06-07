//! Tests for `SecondaryIndex::to_bitmap` and `ids_to_bitmap`.

#[cfg(test)]
mod tests {
    use crate::index::secondary::{JsonValue, SecondaryIndex};
    use parking_lot::RwLock;
    use std::collections::BTreeMap;

    /// Creates a B-tree secondary index with the given entries.
    fn make_btree_index(entries: Vec<(JsonValue, Vec<u64>)>) -> SecondaryIndex {
        let mut tree = BTreeMap::new();
        for (key, ids) in entries {
            tree.insert(key, ids);
        }
        SecondaryIndex::BTree(RwLock::new(tree))
    }

    #[test]
    fn test_to_bitmap_returns_matching_ids() {
        let key = JsonValue::String("tech".to_string());
        let index = make_btree_index(vec![(key.clone(), vec![1, 5, 42])]);

        let bm = index.to_bitmap(&key).expect("test: ids within u32 range");
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(1));
        assert!(bm.contains(5));
        assert!(bm.contains(42));
    }

    #[test]
    fn test_to_bitmap_returns_empty_for_missing_value() {
        let key = JsonValue::String("tech".to_string());
        let missing = JsonValue::String("science".to_string());
        let index = make_btree_index(vec![(key, vec![1, 2, 3])]);

        let bm = index
            .to_bitmap(&missing)
            .expect("test: ids within u32 range");
        assert!(bm.is_empty());
    }

    #[test]
    fn test_to_bitmap_returns_none_when_id_exceeds_u32_max() {
        let key = JsonValue::String("mixed".to_string());
        let large_id = u64::from(u32::MAX) + 1;
        let index = make_btree_index(vec![(key.clone(), vec![10, large_id, 20])]);

        // An ID above u32::MAX cannot be represented in the Roaring bitmap.
        // Rather than silently drop it (which would make a bitmap-only caller
        // miss a real match), `to_bitmap` must signal incompleteness via `None`
        // so the caller falls back to a full scan.
        assert!(
            index.to_bitmap(&key).is_none(),
            "bitmap must be None (incomplete) when an ID exceeds u32::MAX"
        );
    }

    #[test]
    fn test_range_bitmap_returns_none_when_id_exceeds_u32_max() {
        use std::ops::Bound;
        let key = JsonValue::Number(crate::index::secondary::F64Key::from(5.0));
        let large_id = u64::from(u32::MAX) + 7;
        let index = make_btree_index(vec![(key, vec![large_id])]);

        assert!(
            index
                .range_bitmap(Bound::Unbounded, Bound::Unbounded)
                .is_none(),
            "range bitmap must be None when an in-range ID exceeds u32::MAX"
        );
    }

    #[test]
    fn test_to_bitmap_empty_id_list() {
        let key = JsonValue::String("empty".to_string());
        let index = make_btree_index(vec![(key.clone(), vec![])]);

        let bm = index.to_bitmap(&key).expect("test: ids within u32 range");
        assert!(bm.is_empty());
    }

    #[test]
    fn test_to_bitmap_numeric_key() {
        let key = JsonValue::Number(crate::index::secondary::F64Key::from(42.0));
        let index = make_btree_index(vec![(key.clone(), vec![100, 200])]);

        let bm = index.to_bitmap(&key).expect("test: ids within u32 range");
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(100));
        assert!(bm.contains(200));
    }

    #[test]
    fn test_to_bitmap_bool_key() {
        let key = JsonValue::Bool(true);
        let index = make_btree_index(vec![(key.clone(), vec![7, 13])]);

        let bm = index.to_bitmap(&key).expect("test: ids within u32 range");
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(7));
        assert!(bm.contains(13));
    }

    // =====================================================================
    // range_bitmap tests
    // =====================================================================

    /// Creates a numeric B-tree index with prices 10, 20, 30, 40, 50
    /// mapped to IDs 1, 2, 3, 4, 5.
    fn make_price_index() -> SecondaryIndex {
        use crate::index::secondary::F64Key;
        make_btree_index(vec![
            (JsonValue::Number(F64Key::from(10.0)), vec![1]),
            (JsonValue::Number(F64Key::from(20.0)), vec![2]),
            (JsonValue::Number(F64Key::from(30.0)), vec![3]),
            (JsonValue::Number(F64Key::from(40.0)), vec![4]),
            (JsonValue::Number(F64Key::from(50.0)), vec![5]),
        ])
    }

    #[test]
    fn test_range_bitmap_exclusive_lower() {
        use std::ops::Bound;
        let index = make_price_index();
        let key30 = JsonValue::Number(crate::index::secondary::F64Key::from(30.0));

        // (30, +inf) => IDs 4, 5
        let bm = index
            .range_bitmap(Bound::Excluded(&key30), Bound::Unbounded)
            .expect("test: ids within u32 range");
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(4));
        assert!(bm.contains(5));
    }

    #[test]
    fn test_range_bitmap_inclusive_lower() {
        use std::ops::Bound;
        let index = make_price_index();
        let key30 = JsonValue::Number(crate::index::secondary::F64Key::from(30.0));

        // [30, +inf) => IDs 3, 4, 5
        let bm = index
            .range_bitmap(Bound::Included(&key30), Bound::Unbounded)
            .expect("test: ids within u32 range");
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(3));
        assert!(bm.contains(4));
        assert!(bm.contains(5));
    }

    #[test]
    fn test_range_bitmap_exclusive_upper() {
        use std::ops::Bound;
        let index = make_price_index();
        let key30 = JsonValue::Number(crate::index::secondary::F64Key::from(30.0));

        // (-inf, 30) => IDs 1, 2
        let bm = index
            .range_bitmap(Bound::Unbounded, Bound::Excluded(&key30))
            .expect("test: ids within u32 range");
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
    }

    #[test]
    fn test_range_bitmap_inclusive_upper() {
        use std::ops::Bound;
        let index = make_price_index();
        let key30 = JsonValue::Number(crate::index::secondary::F64Key::from(30.0));

        // (-inf, 30] => IDs 1, 2, 3
        let bm = index
            .range_bitmap(Bound::Unbounded, Bound::Included(&key30))
            .expect("test: ids within u32 range");
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
        assert!(bm.contains(3));
    }

    #[test]
    fn test_range_bitmap_closed_interval() {
        use std::ops::Bound;
        let index = make_price_index();
        let key20 = JsonValue::Number(crate::index::secondary::F64Key::from(20.0));
        let key40 = JsonValue::Number(crate::index::secondary::F64Key::from(40.0));

        // [20, 40] => IDs 2, 3, 4
        let bm = index
            .range_bitmap(Bound::Included(&key20), Bound::Included(&key40))
            .expect("test: ids within u32 range");
        assert_eq!(bm.len(), 3);
        assert!(bm.contains(2));
        assert!(bm.contains(3));
        assert!(bm.contains(4));
    }

    #[test]
    fn test_range_bitmap_empty_result() {
        use std::ops::Bound;
        let index = make_price_index();
        let key999 = JsonValue::Number(crate::index::secondary::F64Key::from(999.0));

        // (999, +inf) => empty
        let bm = index
            .range_bitmap(Bound::Excluded(&key999), Bound::Unbounded)
            .expect("test: ids within u32 range");
        assert!(bm.is_empty());
    }

    #[test]
    fn test_range_bitmap_all_values() {
        use std::ops::Bound;
        let index = make_price_index();

        // (-inf, +inf) => all IDs
        let bm = index
            .range_bitmap(Bound::Unbounded, Bound::Unbounded)
            .expect("test: ids within u32 range");
        assert_eq!(bm.len(), 5);
    }

    #[test]
    fn test_range_bitmap_none_when_in_range_id_overflows_u32() {
        use std::ops::Bound;
        let large_id = u64::from(u32::MAX) + 1;
        let index = make_btree_index(vec![
            (
                JsonValue::Number(crate::index::secondary::F64Key::from(10.0)),
                vec![1, large_id],
            ),
            (
                JsonValue::Number(crate::index::secondary::F64Key::from(20.0)),
                vec![2],
            ),
        ]);
        let low_bound = JsonValue::Number(crate::index::secondary::F64Key::from(5.0));

        // (5, +inf) spans key 10 whose posting list contains an id > u32::MAX.
        // The range bitmap cannot represent it, so it must signal `None`
        // (incomplete) instead of silently dropping the high id.
        assert!(
            index
                .range_bitmap(Bound::Excluded(&low_bound), Bound::Unbounded)
                .is_none(),
            "range bitmap must be None when any in-range id overflows u32"
        );

        // A sub-range that excludes the overflowing posting list stays exact.
        let above_overflow_bound = JsonValue::Number(crate::index::secondary::F64Key::from(15.0));
        let bm = index
            .range_bitmap(Bound::Excluded(&above_overflow_bound), Bound::Unbounded)
            .expect("test: ids within u32 range");
        assert_eq!(bm.len(), 1);
        assert!(bm.contains(2));
    }
}
