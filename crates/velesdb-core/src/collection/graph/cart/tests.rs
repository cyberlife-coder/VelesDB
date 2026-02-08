//! Tests for the Compressed Adaptive Radix Tree (C-ART).

use super::super::degree_router::EdgeIndex;
use super::*;

// =========================================================================
// Basic C-ART Tests (TDD: Written first)
// =========================================================================

#[test]
fn test_cart_new_is_empty() {
    let tree = CompressedART::new();
    assert!(tree.is_empty());
    assert_eq!(tree.len(), 0);
}

#[test]
fn test_cart_insert_single() {
    let mut tree = CompressedART::new();
    assert!(tree.insert(42));
    assert_eq!(tree.len(), 1);
    assert!(tree.contains(42));
}

#[test]
fn test_cart_insert_no_duplicates() {
    let mut tree = CompressedART::new();
    assert!(tree.insert(42));
    assert!(!tree.insert(42)); // Duplicate
    assert_eq!(tree.len(), 1);
}

#[test]
fn test_cart_insert_multiple() {
    let mut tree = CompressedART::new();
    for i in 0..100 {
        assert!(tree.insert(i));
    }
    assert_eq!(tree.len(), 100);
    for i in 0..100 {
        assert!(tree.contains(i));
    }
}

#[test]
fn test_cart_remove_existing() {
    let mut tree = CompressedART::new();
    tree.insert(42);
    tree.insert(100);
    tree.insert(7);

    assert!(tree.remove(100));
    assert!(!tree.contains(100));
    assert!(tree.contains(42));
    assert!(tree.contains(7));
    assert_eq!(tree.len(), 2);
}

#[test]
fn test_cart_remove_nonexistent() {
    let mut tree = CompressedART::new();
    tree.insert(42);
    assert!(!tree.remove(999));
    assert_eq!(tree.len(), 1);
}

#[test]
fn test_cart_scan_returns_sorted() {
    let mut tree = CompressedART::new();
    tree.insert(50);
    tree.insert(10);
    tree.insert(30);
    tree.insert(20);
    tree.insert(40);

    let scanned = tree.scan();
    assert_eq!(scanned, vec![10, 20, 30, 40, 50]);
}

#[test]
fn test_cart_large_insertions() {
    let mut tree = CompressedART::new();
    for i in 0..10_000 {
        tree.insert(i);
    }
    assert_eq!(tree.len(), 10_000);

    // Verify all present
    for i in 0..10_000 {
        assert!(tree.contains(i), "Missing value: {i}");
    }
}

#[test]
fn test_cart_random_order_insertions() {
    let mut tree = CompressedART::new();
    let values: Vec<u64> = vec![
        999, 1, 500, 250, 750, 125, 875, 62, 937, 31, 968, 15, 984, 7, 992,
    ];

    for &v in &values {
        tree.insert(v);
    }

    assert_eq!(tree.len(), values.len());
    for &v in &values {
        assert!(tree.contains(v));
    }
}

// =========================================================================
// EdgeIndex Trait Tests
// =========================================================================

#[test]
fn test_cart_edge_index_basic() {
    let mut index = CARTEdgeIndex::new();
    assert!(index.is_empty());

    index.insert(1);
    index.insert(2);
    index.insert(3);

    assert_eq!(index.len(), 3);
    assert!(index.contains(2));
    assert!(!index.contains(99));
}

#[test]
fn test_cart_edge_index_remove() {
    let mut index = CARTEdgeIndex::new();
    index.insert(1);
    index.insert(2);
    index.insert(3);

    assert!(index.remove(2));
    assert!(!index.contains(2));
    assert_eq!(index.len(), 2);
}

#[test]
fn test_cart_edge_index_targets() {
    let mut index = CARTEdgeIndex::new();
    index.insert(30);
    index.insert(10);
    index.insert(20);

    let targets = index.targets();
    // Should be in sorted order
    assert_eq!(targets, vec![10, 20, 30]);
}

#[test]
fn test_cart_edge_index_from_targets() {
    let targets = vec![5, 3, 8, 1, 9];
    let index = CARTEdgeIndex::from_targets(&targets);

    assert_eq!(index.len(), 5);
    for t in targets {
        assert!(index.contains(t));
    }
}

// =========================================================================
// Node Growth Tests
// =========================================================================

#[test]
fn test_cart_node_growth_node4_to_node16() {
    let mut tree = CompressedART::new();
    // Insert 5 values with different first bytes to trigger Node4 -> Node16 growth
    for i in 0..5u64 {
        tree.insert(i << 56); // Different first byte for each
    }
    assert_eq!(tree.len(), 5);
}

#[test]
fn test_cart_node_growth_node16_to_node48() {
    let mut tree = CompressedART::new();
    // Insert 17 values with different first bytes to trigger Node16 -> Node48 growth
    for i in 0..17u64 {
        tree.insert(i << 56);
    }
    assert_eq!(tree.len(), 17);
}

#[test]
fn test_cart_node_growth_node48_to_node256() {
    let mut tree = CompressedART::new();
    // Insert 49 values with different first bytes to trigger Node48 -> Node256 growth
    for i in 0..49u64 {
        tree.insert(i << 56);
    }
    assert_eq!(tree.len(), 49);
}

// =========================================================================
// Stress Tests
// =========================================================================

#[test]
fn test_cart_insert_remove_cycle() {
    let mut tree = CompressedART::new();

    // Insert 1000 values
    for i in 0..1000 {
        tree.insert(i);
    }
    assert_eq!(tree.len(), 1000);

    // Remove even values
    for i in (0..1000).step_by(2) {
        assert!(tree.remove(i));
    }
    assert_eq!(tree.len(), 500);

    // Verify odd values still present
    for i in (1..1000).step_by(2) {
        assert!(tree.contains(i));
    }
}
