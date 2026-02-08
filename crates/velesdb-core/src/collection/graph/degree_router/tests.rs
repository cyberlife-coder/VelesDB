//! Tests for degree-aware storage routing.

use super::*;

#[test]
fn test_vec_edge_index_basic() {
    let mut index = VecEdgeIndex::new();
    assert!(index.is_empty());

    index.insert(1);
    index.insert(2);
    index.insert(3);

    assert_eq!(index.len(), 3);
    assert!(index.contains(2));
    assert!(!index.contains(99));

    assert!(index.remove(2));
    assert!(!index.contains(2));
    assert_eq!(index.len(), 2);
}

#[test]
fn test_vec_edge_index_no_duplicates() {
    let mut index = VecEdgeIndex::new();
    index.insert(1);
    index.insert(1);
    index.insert(1);

    assert_eq!(index.len(), 1);
}

#[test]
fn test_hashset_edge_index_basic() {
    let mut index = HashSetEdgeIndex::new();
    assert!(index.is_empty());

    index.insert(1);
    index.insert(2);
    index.insert(3);

    assert_eq!(index.len(), 3);
    assert!(index.contains(2));
    assert!(!index.contains(99));
}

#[test]
fn test_hashset_from_vec() {
    let mut vec_index = VecEdgeIndex::new();
    for i in 0..50 {
        vec_index.insert(i);
    }

    let hash_index = HashSetEdgeIndex::from_vec(&vec_index);
    assert_eq!(hash_index.len(), 50);

    for i in 0..50 {
        assert!(hash_index.contains(i));
    }
}

#[test]
fn test_degree_adaptive_storage_promotion() {
    let mut storage = DegreeAdaptiveStorage::new();
    assert!(!storage.is_high_degree());

    // Fill with data
    for i in 0..50 {
        storage.insert(i);
    }
    assert!(!storage.is_high_degree());

    // Promote
    storage.promote_to_high_degree();
    assert!(storage.is_high_degree());

    // Data should still be there
    assert_eq!(storage.len(), 50);
    for i in 0..50 {
        assert!(storage.contains(i));
    }
}

#[test]
fn test_degree_router_auto_promotion() {
    let mut router = DegreeRouter::with_threshold(10);
    assert!(!router.is_high_degree());
    assert_eq!(router.promotion_count(), 0);

    // Insert below threshold
    for i in 0..10 {
        router.insert(i);
    }
    assert!(!router.is_high_degree());

    // Insert one more to trigger promotion
    router.insert(100);
    assert!(router.is_high_degree());
    assert_eq!(router.promotion_count(), 1);
    assert_eq!(router.len(), 11);
}

#[test]
fn test_degree_router_stays_high_degree() {
    let mut router = DegreeRouter::with_threshold(5);

    // Trigger promotion
    for i in 0..10 {
        router.insert(i);
    }
    assert!(router.is_high_degree());

    // Remove items below threshold
    for i in 0..8 {
        router.remove(i);
    }

    // Should stay high-degree (no demotion)
    assert!(router.is_high_degree());
    assert_eq!(router.len(), 2);
}

#[test]
fn test_degree_router_default_threshold() {
    let router = DegreeRouter::new();
    assert_eq!(router.threshold(), DEFAULT_DEGREE_THRESHOLD);
}

#[test]
fn test_degree_router_cart_promotion() {
    let mut router = DegreeRouter::with_threshold(10);

    // Insert enough to trigger Vec -> HashSet -> C-ART
    for i in 0..1002 {
        router.insert(i);
    }

    // Should have promoted twice: Vec->HashSet and HashSet->C-ART
    assert!(router.storage.is_very_high_degree());
    assert_eq!(router.promotion_count(), 2);
    assert_eq!(router.len(), 1002);

    // Verify all values still accessible
    for i in 0..1002 {
        assert!(router.contains(i), "Missing value: {i}");
    }
}

#[test]
fn test_degree_adaptive_storage_cart_promotion() {
    let mut storage = DegreeAdaptiveStorage::new();

    // Fill with data
    for i in 0..500 {
        storage.insert(i);
    }

    // Promote to HashSet
    storage.promote_to_high_degree();
    assert!(storage.is_high_degree());
    assert!(!storage.is_very_high_degree());

    // Add more data
    for i in 500..1000 {
        storage.insert(i);
    }

    // Promote to C-ART
    storage.promote_to_cart();
    assert!(storage.is_very_high_degree());
    assert_eq!(storage.len(), 1000);

    // Verify data integrity
    for i in 0..1000 {
        assert!(storage.contains(i));
    }
}
