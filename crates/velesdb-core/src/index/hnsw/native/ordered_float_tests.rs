//! Tests for OrderedFloat module.

use super::ordered_float::OrderedFloat;
use std::cmp::Ordering;

#[test]
fn test_ordered_float_eq() {
    let a = OrderedFloat(1.0);
    let b = OrderedFloat(1.0);
    assert_eq!(a, b);
}

#[test]
fn test_ordered_float_ne() {
    let a = OrderedFloat(1.0);
    let b = OrderedFloat(2.0);
    assert_ne!(a, b);
}

#[test]
fn test_ordered_float_ord_less() {
    let a = OrderedFloat(1.0);
    let b = OrderedFloat(2.0);
    assert_eq!(a.cmp(&b), Ordering::Less);
    assert!(a < b);
}

#[test]
fn test_ordered_float_ord_greater() {
    let a = OrderedFloat(3.0);
    let b = OrderedFloat(2.0);
    assert_eq!(a.cmp(&b), Ordering::Greater);
    assert!(a > b);
}

#[test]
fn test_ordered_float_ord_equal() {
    let a = OrderedFloat(2.0);
    let b = OrderedFloat(2.0);
    assert_eq!(a.cmp(&b), Ordering::Equal);
}

#[test]
fn test_ordered_float_negative() {
    let a = OrderedFloat(-1.0);
    let b = OrderedFloat(1.0);
    assert!(a < b);
}

#[test]
fn test_ordered_float_zero() {
    let a = OrderedFloat(0.0);
    let b = OrderedFloat(-0.0);
    // IEEE 754 total ordering: -0.0 < +0.0 (different bit representations)
    // This is the correct behavior for total_cmp
    assert_eq!(a.cmp(&b), Ordering::Greater);
    assert_eq!(b.cmp(&a), Ordering::Less);
    // But same value zeros should be equal
    assert_eq!(OrderedFloat(0.0).cmp(&OrderedFloat(0.0)), Ordering::Equal);
    assert_eq!(OrderedFloat(-0.0).cmp(&OrderedFloat(-0.0)), Ordering::Equal);
}

#[test]
fn test_ordered_float_in_binary_heap() {
    use std::collections::BinaryHeap;

    let mut heap = BinaryHeap::new();
    heap.push(OrderedFloat(3.0));
    heap.push(OrderedFloat(1.0));
    heap.push(OrderedFloat(2.0));

    // BinaryHeap is max-heap
    assert_eq!(heap.pop(), Some(OrderedFloat(3.0)));
    assert_eq!(heap.pop(), Some(OrderedFloat(2.0)));
    assert_eq!(heap.pop(), Some(OrderedFloat(1.0)));
}

#[test]
fn test_ordered_float_sorting() {
    let mut floats = [
        OrderedFloat(3.0),
        OrderedFloat(1.0),
        OrderedFloat(2.0),
        OrderedFloat(-1.0),
    ];
    floats.sort();

    assert_eq!(floats[0], OrderedFloat(-1.0));
    assert_eq!(floats[1], OrderedFloat(1.0));
    assert_eq!(floats[2], OrderedFloat(2.0));
    assert_eq!(floats[3], OrderedFloat(3.0));
}

#[test]
#[allow(clippy::float_cmp)]
fn test_ordered_float_inner_value() {
    let of = OrderedFloat(42.5);
    assert_eq!(of.0, 42.5);
}
