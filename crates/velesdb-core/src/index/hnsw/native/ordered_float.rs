//! Ordered float wrapper for use in `BinaryHeap`.
//!
//! Provides IEEE 754 total ordering for f32 values, including proper NaN handling.
//! Uses `f32::total_cmp` which defines: -NaN < -∞ < ... < -0 < +0 < ... < +∞ < +NaN

use std::cmp::Ordering;

/// Wrapper for f32 to implement Ord for `BinaryHeap`.
///
/// Uses `f32::total_cmp` for IEEE 754 total ordering, ensuring Ord/Eq/PartialEq
/// consistency even with NaN values. This prevents heap corruption during HNSW search.
#[derive(Debug, Clone, Copy)]
pub(super) struct OrderedFloat(pub f32);

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        // Use bit comparison for consistency with total_cmp
        // This ensures NaN == NaN (same bits) and -0.0 != +0.0
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> Ordering {
        // f32::total_cmp provides IEEE 754 total ordering:
        // -NaN < -∞ < -max < ... < -0 < +0 < ... < +max < +∞ < +NaN
        self.0.total_cmp(&other.0)
    }
}
