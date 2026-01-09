//! Tests for `PostingList` adaptive posting list.

use super::posting_list::{PostingList, PROMOTION_THRESHOLD};

#[test]
fn test_new_creates_small() {
    let pl = PostingList::new();
    assert!(!pl.is_large());
    assert!(pl.is_empty());
    assert_eq!(pl.len(), 0);
}

#[test]
fn test_with_capacity_small() {
    let pl = PostingList::with_capacity(100);
    assert!(!pl.is_large());
}

#[test]
fn test_with_capacity_large() {
    let pl = PostingList::with_capacity(PROMOTION_THRESHOLD);
    assert!(pl.is_large());
}

#[test]
fn test_insert_and_contains() {
    let mut pl = PostingList::new();
    assert!(!pl.contains(42));

    assert!(pl.insert(42));
    assert!(pl.contains(42));
    assert_eq!(pl.len(), 1);

    // Duplicate insert returns false
    assert!(!pl.insert(42));
    assert_eq!(pl.len(), 1);
}

#[test]
fn test_remove() {
    let mut pl = PostingList::new();
    pl.insert(1);
    pl.insert(2);
    pl.insert(3);

    assert!(pl.remove(2));
    assert!(!pl.contains(2));
    assert_eq!(pl.len(), 2);

    // Remove non-existent returns false
    assert!(!pl.remove(999));
}

#[test]
fn test_automatic_promotion_to_large() {
    let mut pl = PostingList::new();
    assert!(!pl.is_large());

    // Insert enough elements to trigger promotion
    for i in 0..PROMOTION_THRESHOLD as u32 {
        pl.insert(i);
    }

    assert!(pl.is_large(), "Should promote to Large after {} inserts", PROMOTION_THRESHOLD);
    assert_eq!(pl.len(), PROMOTION_THRESHOLD);

    // Verify all elements are still accessible
    for i in 0..PROMOTION_THRESHOLD as u32 {
        assert!(pl.contains(i), "Should contain {} after promotion", i);
    }
}

#[test]
fn test_iter_small() {
    let mut pl = PostingList::new();
    pl.insert(1);
    pl.insert(2);
    pl.insert(3);

    let mut collected: Vec<u32> = pl.iter().collect();
    collected.sort();
    assert_eq!(collected, vec![1, 2, 3]);
}

#[test]
fn test_iter_large() {
    let mut pl = PostingList::with_capacity(PROMOTION_THRESHOLD);
    pl.insert(100);
    pl.insert(200);
    pl.insert(300);

    let mut collected: Vec<u32> = pl.iter().collect();
    collected.sort();
    assert_eq!(collected, vec![100, 200, 300]);
}

#[test]
fn test_union_small_small() {
    let mut a = PostingList::new();
    a.insert(1);
    a.insert(2);

    let mut b = PostingList::new();
    b.insert(2);
    b.insert(3);

    let union = a.union(&b);
    assert!(!union.is_large()); // Still small

    let mut collected: Vec<u32> = union.iter().collect();
    collected.sort();
    assert_eq!(collected, vec![1, 2, 3]);
}

#[test]
fn test_union_promotes_when_large_combined() {
    let mut a = PostingList::new();
    for i in 0..600 {
        a.insert(i);
    }

    let mut b = PostingList::new();
    for i in 500..1100 {
        b.insert(i);
    }

    let union = a.union(&b);
    assert!(union.is_large(), "Union should promote to Large when combined size >= threshold");
    assert_eq!(union.len(), 1100); // 0..1100 unique
}

#[test]
fn test_union_large_large() {
    let mut a = PostingList::with_capacity(PROMOTION_THRESHOLD);
    for i in 0..500 {
        a.insert(i);
    }

    let mut b = PostingList::with_capacity(PROMOTION_THRESHOLD);
    for i in 250..750 {
        b.insert(i);
    }

    let union = a.union(&b);
    assert!(union.is_large());
    assert_eq!(union.len(), 750); // 0..750 unique
}

#[test]
fn test_union_small_large() {
    let mut small = PostingList::new();
    small.insert(1);
    small.insert(2);

    let mut large = PostingList::with_capacity(PROMOTION_THRESHOLD);
    large.insert(2);
    large.insert(3);

    let union = small.union(&large);
    assert!(union.is_large()); // Inherits Large from operand

    let mut collected: Vec<u32> = union.iter().collect();
    collected.sort();
    assert_eq!(collected, vec![1, 2, 3]);
}

#[test]
fn test_iter_exact_size() {
    let mut pl = PostingList::new();
    for i in 0..50 {
        pl.insert(i);
    }

    let iter = pl.iter();
    assert_eq!(iter.len(), 50);
}

#[test]
fn test_clone() {
    let mut pl = PostingList::new();
    pl.insert(1);
    pl.insert(2);

    let cloned = pl.clone();
    assert_eq!(cloned.len(), 2);
    assert!(cloned.contains(1));
    assert!(cloned.contains(2));
}

// ============================================================================
// Performance-oriented tests for large datasets
// ============================================================================

#[test]
fn test_large_dataset_insert_performance() {
    // Test that Large representation handles 100K+ docs efficiently
    let mut pl = PostingList::with_capacity(PROMOTION_THRESHOLD);

    for i in 0..100_000 {
        pl.insert(i);
    }

    assert!(pl.is_large());
    assert_eq!(pl.len(), 100_000);

    // Spot check
    assert!(pl.contains(0));
    assert!(pl.contains(50_000));
    assert!(pl.contains(99_999));
    assert!(!pl.contains(100_000));
}

#[test]
fn test_large_union_performance() {
    // Test union of two large posting lists
    let mut a = PostingList::with_capacity(PROMOTION_THRESHOLD);
    let mut b = PostingList::with_capacity(PROMOTION_THRESHOLD);

    for i in 0..50_000 {
        a.insert(i);
    }
    for i in 25_000..75_000 {
        b.insert(i);
    }

    let union = a.union(&b);
    assert_eq!(union.len(), 75_000); // 0..75000
}

#[test]
fn test_sparse_ids() {
    // Test with sparse, non-contiguous IDs (common in real-world)
    let mut pl = PostingList::new();

    let ids: Vec<u32> = (0..2000).map(|i| i * 1000).collect();
    for &id in &ids {
        pl.insert(id);
    }

    assert!(pl.is_large());
    assert_eq!(pl.len(), 2000);

    for &id in &ids {
        assert!(pl.contains(id));
    }
}
