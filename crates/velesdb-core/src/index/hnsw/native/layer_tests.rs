//! Tests for Layer module.

use super::layer::{Layer, NodeId};

#[test]
fn test_layer_new_empty() {
    let layer = Layer::new(0);
    assert_eq!(layer.get_neighbors(0), Vec::<NodeId>::new());
}

#[test]
fn test_layer_new_with_capacity() {
    let layer = Layer::new(10);
    // All nodes should have empty neighbor lists
    for i in 0..10 {
        assert!(layer.get_neighbors(i).is_empty());
    }
}

#[test]
fn test_layer_set_and_get_neighbors() {
    let layer = Layer::new(5);
    let neighbors = vec![1, 2, 3];

    layer.set_neighbors(0, neighbors.clone());
    assert_eq!(layer.get_neighbors(0), neighbors);
}

#[test]
fn test_layer_get_neighbors_out_of_bounds() {
    let layer = Layer::new(5);
    // Should return empty vec for out of bounds
    assert!(layer.get_neighbors(100).is_empty());
}

#[test]
fn test_layer_set_neighbors_out_of_bounds() {
    let layer = Layer::new(5);
    // Should not panic, just no-op
    layer.set_neighbors(100, vec![1, 2, 3]);
    assert!(layer.get_neighbors(100).is_empty());
}

#[test]
fn test_layer_ensure_capacity_grows() {
    let mut layer = Layer::new(5);
    assert!(layer.get_neighbors(10).is_empty());

    layer.ensure_capacity(10);
    layer.set_neighbors(10, vec![1, 2]);
    assert_eq!(layer.get_neighbors(10), vec![1, 2]);
}

#[test]
fn test_layer_ensure_capacity_no_shrink() {
    let mut layer = Layer::new(10);
    layer.set_neighbors(5, vec![1, 2, 3]);

    // ensure_capacity with smaller value should not affect existing data
    layer.ensure_capacity(3);
    assert_eq!(layer.get_neighbors(5), vec![1, 2, 3]);
}

#[test]
fn test_layer_multiple_nodes() {
    let layer = Layer::new(3);

    layer.set_neighbors(0, vec![1, 2]);
    layer.set_neighbors(1, vec![0, 2]);
    layer.set_neighbors(2, vec![0, 1]);

    assert_eq!(layer.get_neighbors(0), vec![1, 2]);
    assert_eq!(layer.get_neighbors(1), vec![0, 2]);
    assert_eq!(layer.get_neighbors(2), vec![0, 1]);
}

#[test]
fn test_layer_overwrite_neighbors() {
    let layer = Layer::new(5);

    layer.set_neighbors(0, vec![1, 2, 3]);
    assert_eq!(layer.get_neighbors(0), vec![1, 2, 3]);

    layer.set_neighbors(0, vec![4, 5]);
    assert_eq!(layer.get_neighbors(0), vec![4, 5]);
}

#[test]
fn test_layer_empty_neighbors() {
    let layer = Layer::new(5);

    layer.set_neighbors(0, vec![1, 2, 3]);
    layer.set_neighbors(0, vec![]);

    assert!(layer.get_neighbors(0).is_empty());
}
