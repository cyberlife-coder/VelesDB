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

        let bm = index.to_bitmap(&key);
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

        let bm = index.to_bitmap(&missing);
        assert!(bm.is_empty());
    }

    #[test]
    fn test_to_bitmap_skips_ids_exceeding_u32_max() {
        let key = JsonValue::String("mixed".to_string());
        let large_id = u64::from(u32::MAX) + 1;
        let index = make_btree_index(vec![(key.clone(), vec![10, large_id, 20])]);

        let bm = index.to_bitmap(&key);
        // Only u32-representable IDs should be in the bitmap
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(10));
        assert!(bm.contains(20));
    }

    #[test]
    fn test_to_bitmap_empty_id_list() {
        let key = JsonValue::String("empty".to_string());
        let index = make_btree_index(vec![(key.clone(), vec![])]);

        let bm = index.to_bitmap(&key);
        assert!(bm.is_empty());
    }

    #[test]
    fn test_to_bitmap_numeric_key() {
        let key = JsonValue::Number(crate::index::secondary::F64Key::from(42.0));
        let index = make_btree_index(vec![(key.clone(), vec![100, 200])]);

        let bm = index.to_bitmap(&key);
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(100));
        assert!(bm.contains(200));
    }

    #[test]
    fn test_to_bitmap_bool_key() {
        let key = JsonValue::Bool(true);
        let index = make_btree_index(vec![(key.clone(), vec![7, 13])]);

        let bm = index.to_bitmap(&key);
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
        let bm = index.range_bitmap(Bound::Excluded(&key30), Bound::Unbounded);
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
        let bm = index.range_bitmap(Bound::Included(&key30), Bound::Unbounded);
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
        let bm = index.range_bitmap(Bound::Unbounded, Bound::Excluded(&key30));
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
        let bm = index.range_bitmap(Bound::Unbounded, Bound::Included(&key30));
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
        let bm = index.range_bitmap(Bound::Included(&key20), Bound::Included(&key40));
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
        let bm = index.range_bitmap(Bound::Excluded(&key999), Bound::Unbounded);
        assert!(bm.is_empty());
    }

    #[test]
    fn test_range_bitmap_all_values() {
        use std::ops::Bound;
        let index = make_price_index();

        // (-inf, +inf) => all IDs
        let bm = index.range_bitmap(Bound::Unbounded, Bound::Unbounded);
        assert_eq!(bm.len(), 5);
    }

    #[test]
    fn test_range_bitmap_skips_u64_overflow() {
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
        let key5 = JsonValue::Number(crate::index::secondary::F64Key::from(5.0));

        // (5, +inf) => IDs 1, 2 (large_id skipped)
        let bm = index.range_bitmap(Bound::Excluded(&key5), Bound::Unbounded);
        assert_eq!(bm.len(), 2);
        assert!(bm.contains(1));
        assert!(bm.contains(2));
    }
}
