use super::*;

#[test]
fn test_clustered_index_basic() {
    let mut index = ClusteredIndex::new();

    index.insert(1, 10);
    index.insert(1, 20);
    index.insert(1, 30);

    assert_eq!(index.get_neighbors(1).len(), 3);
    assert!(index.contains(1, 10));
    assert!(index.contains(1, 20));
    assert!(index.contains(1, 30));
}

#[test]
fn test_clustered_index_multiple_nodes() {
    let mut index = ClusteredIndex::new();

    index.insert(1, 10);
    index.insert(1, 20);
    index.insert(2, 100);
    index.insert(2, 200);
    index.insert(3, 1000);

    assert_eq!(index.node_count(), 3);
    assert_eq!(index.edge_count(), 5);
    assert_eq!(index.neighbor_count(1), 2);
    assert_eq!(index.neighbor_count(2), 2);
    assert_eq!(index.neighbor_count(3), 1);
}

#[test]
fn test_clustered_index_no_duplicates() {
    let mut index = ClusteredIndex::new();

    index.insert(1, 10);
    index.insert(1, 10);
    index.insert(1, 10);

    assert_eq!(index.neighbor_count(1), 1);
}

#[test]
fn test_clustered_index_remove() {
    let mut index = ClusteredIndex::new();

    index.insert(1, 10);
    index.insert(1, 20);
    index.insert(1, 30);

    assert!(index.remove(1, 20));
    assert!(!index.contains(1, 20));
    assert_eq!(index.neighbor_count(1), 2);

    assert!(!index.remove(1, 99)); // Not present
}

#[test]
fn test_clustered_index_remove_node() {
    let mut index = ClusteredIndex::new();

    index.insert(1, 10);
    index.insert(1, 20);
    index.insert(2, 100);

    index.remove_node(1);

    assert_eq!(index.node_count(), 1);
    assert_eq!(index.neighbor_count(1), 0);
    assert_eq!(index.neighbor_count(2), 1);
}

#[test]
fn test_clustered_index_compaction() {
    let mut index = ClusteredIndex::new();

    // Create some data
    for i in 0..10 {
        for j in 0..5 {
            index.insert(i, j * 100);
        }
    }

    // Remove some to create fragmentation
    for i in 0..5 {
        index.remove_node(i);
    }

    let frag_before = index.fragmentation();
    assert!(frag_before > 0.0);

    index.compact();

    assert!(index.fragmentation().abs() < f64::EPSILON);
    assert_eq!(index.node_count(), 5);
}

#[test]
fn test_clustered_index_slot_reuse() {
    let mut index = ClusteredIndex::new();

    // Fill some data
    index.insert(1, 10);
    index.insert(1, 20);

    // Remove and add - should reuse slots
    index.remove_node(1);
    index.insert(2, 100);

    assert_eq!(index.node_count(), 1);
    assert!(index.contains(2, 100));
}
