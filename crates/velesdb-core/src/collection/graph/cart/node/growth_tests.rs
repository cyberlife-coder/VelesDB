//! Tests for C-ART node growth operations (Node4→Node16→Node48→Node256).

use super::CARTNode;

/// Helper: builds a leaf node containing a single value.
fn leaf(value: u64) -> Box<CARTNode> {
    Box::new(CARTNode::new_leaf(value))
}

// =========================================================================
// grow_to_node16: Node4 → Node16
// =========================================================================

#[test]
fn grow_node4_to_node16_preserves_children() {
    let node = CARTNode::Node4 {
        num_children: 4,
        keys: [30, 10, 40, 20],
        children: [
            Some(leaf(300)),
            Some(leaf(100)),
            Some(leaf(400)),
            Some(leaf(200)),
        ],
    };

    let grown = node.grow_to_node16();

    // Must be Node16 with keys sorted and children re-ordered to match
    match &grown {
        CARTNode::Node16 {
            num_children,
            keys,
            children,
        } => {
            assert_eq!(*num_children, 4);
            // Keys are sorted by the growth function
            assert_eq!(&keys[..4], &[10, 20, 30, 40]);
            // Each key's child must carry the matching leaf value
            for (i, expected_val) in [100u64, 200, 300, 400].iter().enumerate() {
                let child = children[i].as_ref().unwrap();
                let mut vals = Vec::new();
                child.collect_all(&mut vals);
                assert_eq!(vals, vec![*expected_val]);
            }
            // Remaining slots must be None
            for slot in &children[4..] {
                assert!(slot.is_none());
            }
        }
        other => panic!("Expected Node16, got {other:?}"),
    }
}

#[test]
fn grow_node4_to_node16_partial_fill() {
    let node = CARTNode::Node4 {
        num_children: 2,
        keys: [5, 3, 0, 0],
        children: [Some(leaf(50)), Some(leaf(30)), None, None],
    };

    let grown = node.grow_to_node16();

    match &grown {
        CARTNode::Node16 {
            num_children,
            keys,
            children,
        } => {
            assert_eq!(*num_children, 2);
            assert_eq!(&keys[..2], &[3, 5]);
            assert!(children[0].is_some());
            assert!(children[1].is_some());
            for slot in &children[2..] {
                assert!(slot.is_none());
            }
        }
        other => panic!("Expected Node16, got {other:?}"),
    }
}

// =========================================================================
// grow_to_node48: Node16 → Node48
// =========================================================================

#[test]
fn grow_node16_to_node48_preserves_children() {
    let mut keys = [0u8; 16];
    let mut children: [Option<Box<CARTNode>>; 16] = Default::default();

    // Fill all 16 slots with distinct keys
    for i in 0u8..16 {
        let key = i * 10; // 0, 10, 20, ... 150
        keys[i as usize] = key;
        children[i as usize] = Some(leaf(u64::from(key) * 100));
    }

    let node = CARTNode::Node16 {
        num_children: 16,
        keys,
        children,
    };
    let grown = node.grow_to_node48();

    match &grown {
        CARTNode::Node48 {
            num_children,
            keys: idx,
            children: slots,
        } => {
            assert_eq!(*num_children, 16);
            // For each original key byte, the index must map to a valid slot
            for i in 0u8..16 {
                let key_byte = i * 10;
                let slot = idx[key_byte as usize];
                assert_ne!(slot, 255, "key {key_byte} should have a slot");
                let child = slots[slot as usize].as_ref().unwrap();
                let mut vals = Vec::new();
                child.collect_all(&mut vals);
                assert_eq!(vals, vec![u64::from(key_byte) * 100]);
            }
        }
        other => panic!("Expected Node48, got {other:?}"),
    }
}

#[test]
fn grow_node16_to_node48_empty_slots_are_255() {
    let mut keys = [0u8; 16];
    let mut children: [Option<Box<CARTNode>>; 16] = Default::default();

    // Only 3 children
    keys[0] = 5;
    keys[1] = 100;
    keys[2] = 200;
    children[0] = Some(leaf(1));
    children[1] = Some(leaf(2));
    children[2] = Some(leaf(3));

    let node = CARTNode::Node16 {
        num_children: 3,
        keys,
        children,
    };
    let grown = node.grow_to_node48();

    match &grown {
        CARTNode::Node48 {
            num_children, keys, ..
        } => {
            assert_eq!(*num_children, 3);
            // Bytes without a child must map to 255
            let occupied: [usize; 3] = [5, 100, 200];
            for (byte, &key_val) in keys.iter().enumerate() {
                if occupied.contains(&byte) {
                    assert_ne!(key_val, 255);
                } else {
                    assert_eq!(key_val, 255, "byte {byte} should be empty (255)");
                }
            }
        }
        other => panic!("Expected Node48, got {other:?}"),
    }
}

// =========================================================================
// grow_to_node256: Node48 → Node256
// =========================================================================

#[test]
fn grow_node48_to_node256_preserves_children() {
    let mut keys = [255u8; 256];
    let mut children: [Option<Box<CARTNode>>; 48] = std::array::from_fn(|_| None);

    // Place 5 children at known byte positions
    let entries: [(u8, u64); 5] = [(0, 10), (42, 420), (128, 1280), (200, 2000), (255, 2550)];
    for (slot, &(byte, val)) in entries.iter().enumerate() {
        keys[byte as usize] = slot as u8;
        children[slot] = Some(leaf(val));
    }

    let node = CARTNode::Node48 {
        num_children: 5,
        keys,
        children,
    };
    let grown = node.grow_to_node256();

    match &grown {
        CARTNode::Node256 {
            num_children,
            children,
        } => {
            assert_eq!(*num_children, 5);
            for &(byte, val) in &entries {
                let child = children[byte as usize].as_ref().unwrap();
                let mut vals = Vec::new();
                child.collect_all(&mut vals);
                assert_eq!(vals, vec![val], "child at byte {byte} mismatch");
            }
            // Count populated slots
            let populated = children.iter().filter(|c| c.is_some()).count();
            assert_eq!(populated, 5);
        }
        other => panic!("Expected Node256, got {other:?}"),
    }
}

// =========================================================================
// Wrong-type calls: should clone (no-op)
// =========================================================================

#[test]
fn grow_to_node16_on_node16_returns_clone() {
    let node = CARTNode::Node16 {
        num_children: 1,
        keys: {
            let mut k = [0u8; 16];
            k[0] = 42;
            k
        },
        children: {
            let mut c: [Option<Box<CARTNode>>; 16] = Default::default();
            c[0] = Some(leaf(99));
            c
        },
    };

    let result = node.grow_to_node16();
    assert!(matches!(
        result,
        CARTNode::Node16 {
            num_children: 1,
            ..
        }
    ));
}

#[test]
fn grow_to_node16_on_leaf_returns_clone() {
    let node = CARTNode::new_leaf(7);
    let result = node.grow_to_node16();
    assert!(matches!(result, CARTNode::Leaf { .. }));
}

#[test]
fn grow_to_node48_on_node4_returns_clone() {
    let node = CARTNode::Node4 {
        num_children: 1,
        keys: [10, 0, 0, 0],
        children: [Some(leaf(1)), None, None, None],
    };

    let result = node.grow_to_node48();
    assert!(matches!(
        result,
        CARTNode::Node4 {
            num_children: 1,
            ..
        }
    ));
}

#[test]
fn grow_to_node256_on_node16_returns_clone() {
    let node = CARTNode::Node16 {
        num_children: 0,
        keys: [0u8; 16],
        children: Default::default(),
    };

    let result = node.grow_to_node256();
    assert!(matches!(
        result,
        CARTNode::Node16 {
            num_children: 0,
            ..
        }
    ));
}
