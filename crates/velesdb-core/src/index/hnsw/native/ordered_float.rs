//! Ordered float wrapper for use in `BinaryHeap`.
//!
//! Provides total ordering for f32 values, handling NaN as equal.

use std::cmp::Ordering;

/// Wrapper for f32 to implement Ord for `BinaryHeap`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct OrderedFloat(pub f32);

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.partial_cmp(&other.0).unwrap_or(Ordering::Equal)
    }
}
