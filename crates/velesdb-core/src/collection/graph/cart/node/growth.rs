//! Node growth operations for C-ART.
//!
//! Handles promotion from smaller to larger node types:
//! - Node4 → Node16
//! - Node16 → Node48
//! - Node48 → Node256

// SAFETY: Numeric casts in C-ART node operations are intentional:
// - usize->u8 for child indices: C-ART nodes have max 256 children, indices fit in u8
// - Node types enforce size limits (Node4=4, Node16=16, Node48=48, Node256=256)
// - All index values are validated against node capacity before casting
#![allow(clippy::cast_possible_truncation)]

use super::CARTNode;

impl CARTNode {
    /// Grows Node4 to Node16.
    pub(super) fn grow_to_node16(&self) -> Self {
        match self {
            Self::Node4 {
                num_children,
                keys,
                children,
            } => {
                let mut new_keys = [0u8; 16];
                let mut new_children: [Option<Box<CARTNode>>; 16] = Default::default();

                // Copy and sort
                let n = *num_children as usize;
                let mut indices: Vec<usize> = (0..n).collect();
                indices.sort_by_key(|&i| keys[i]);

                for (new_idx, &old_idx) in indices.iter().enumerate() {
                    new_keys[new_idx] = keys[old_idx];
                    new_children[new_idx].clone_from(&children[old_idx]);
                }

                Self::Node16 {
                    num_children: *num_children,
                    keys: new_keys,
                    children: new_children,
                }
            }
            _ => self.clone(),
        }
    }

    /// Grows Node16 to Node48.
    pub(super) fn grow_to_node48(&self) -> Self {
        match self {
            Self::Node16 {
                num_children,
                keys,
                children,
            } => {
                let mut new_keys = [255u8; 256];
                let mut new_children: [Option<Box<CARTNode>>; 48] = std::array::from_fn(|_| None);

                for i in 0..*num_children as usize {
                    new_keys[keys[i] as usize] = i as u8;
                    new_children[i].clone_from(&children[i]);
                }

                Self::Node48 {
                    num_children: *num_children,
                    keys: new_keys,
                    children: new_children,
                }
            }
            _ => self.clone(),
        }
    }

    /// Grows Node48 to Node256.
    pub(super) fn grow_to_node256(&self) -> Self {
        match self {
            Self::Node48 {
                num_children,
                keys,
                children,
            } => {
                let mut new_children: [Option<Box<CARTNode>>; 256] = std::array::from_fn(|_| None);

                for (byte, &slot) in keys.iter().enumerate() {
                    if slot != 255 {
                        new_children[byte].clone_from(&children[slot as usize]);
                    }
                }

                Self::Node256 {
                    num_children: u16::from(*num_children),
                    children: new_children,
                }
            }
            _ => self.clone(),
        }
    }
}
