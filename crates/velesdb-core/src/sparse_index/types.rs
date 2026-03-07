//! Core sparse vector types: `SparseVector`, `PostingEntry`, and `ScoredDoc`.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// The canonical name used to identify the default (unnamed) sparse index.
///
/// When a point's `sparse_vectors` map has an entry keyed by `""`, it is
/// stored under this index. Queries that omit the `USING` clause also resolve
/// to this name. Using this constant avoids magic empty-string literals
/// scattered across the codebase.
pub const DEFAULT_SPARSE_INDEX_NAME: &str = "";

/// A sparse vector represented as sorted parallel arrays of indices and values.
///
/// Invariants maintained at construction:
/// - `indices` is sorted in ascending order with no duplicates.
/// - Every corresponding `values` entry is nonzero.
/// - `indices.len() == values.len()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SparseVector {
    /// Sorted unique dimension indices (ascending).
    pub indices: Vec<u32>,
    /// Weights corresponding to each index. Never zero.
    pub values: Vec<f32>,
}

impl SparseVector {
    /// Constructs a `SparseVector` from unsorted `(index, weight)` pairs.
    ///
    /// - Sorts by index.
    /// - Merges duplicate indices by summing their weights.
    /// - Filters entries whose final weight is effectively zero.
    #[must_use]
    pub fn new(mut pairs: Vec<(u32, f32)>) -> Self {
        if pairs.is_empty() {
            return Self {
                indices: Vec::new(),
                values: Vec::new(),
            };
        }

        // Sort by index
        pairs.sort_by_key(|&(idx, _)| idx);

        let mut indices = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len());

        let mut current_idx = pairs[0].0;
        let mut current_val = pairs[0].1;

        for &(idx, val) in &pairs[1..] {
            if idx == current_idx {
                current_val += val;
            } else {
                // Flush previous.
                // Discard entries with exactly zero weight. We use strict zero comparison
                // rather than an epsilon threshold to avoid discarding legitimately small
                // non-zero weights (e.g., sub-unit TF-IDF or SPLADE scores).
                if current_val != 0.0 {
                    indices.push(current_idx);
                    values.push(current_val);
                }
                current_idx = idx;
                current_val = val;
            }
        }
        // Flush last
        if current_val != 0.0 {
            indices.push(current_idx);
            values.push(current_val);
        }

        Self { indices, values }
    }

    /// Constructs a `SparseVector` from pre-sorted, unique, nonzero arrays.
    ///
    /// # Safety (debug-only)
    ///
    /// In debug builds, asserts that `indices` is sorted with no duplicates
    /// and that `indices.len() == values.len()`.
    #[must_use]
    pub fn from_sorted_unchecked(indices: Vec<u32>, values: Vec<f32>) -> Self {
        debug_assert_eq!(
            indices.len(),
            values.len(),
            "indices and values must have equal length"
        );
        debug_assert!(
            indices.windows(2).all(|w| w[0] < w[1]),
            "indices must be sorted and unique"
        );
        Self { indices, values }
    }

    /// Returns the number of nonzero entries.
    #[must_use]
    pub fn nnz(&self) -> usize {
        self.indices.len()
    }

    /// Returns `true` if this sparse vector has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Computes the dot product with another sparse vector using merge-join.
    ///
    /// Runs in O(n + m) where n and m are the nonzero counts of each vector.
    #[must_use]
    pub fn dot(&self, other: &Self) -> f32 {
        let mut i = 0;
        let mut j = 0;
        let mut sum = 0.0_f32;

        while i < self.indices.len() && j < other.indices.len() {
            match self.indices[i].cmp(&other.indices[j]) {
                Ordering::Less => i += 1,
                Ordering::Greater => j += 1,
                Ordering::Equal => {
                    sum += self.values[i] * other.values[j];
                    i += 1;
                    j += 1;
                }
            }
        }

        sum
    }
}

/// A single entry in a posting list: document ID and its weight for that term.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PostingEntry {
    /// Document (point) identifier.
    pub doc_id: u64,
    /// Term weight for this document.
    pub weight: f32,
}

/// A scored document result from sparse search.
#[derive(Debug, Clone)]
pub struct ScoredDoc {
    /// Relevance score (higher is better).
    pub score: f32,
    /// Document (point) identifier.
    pub doc_id: u64,
}

impl PartialEq for ScoredDoc {
    fn eq(&self, other: &Self) -> bool {
        self.score.total_cmp(&other.score) == Ordering::Equal && self.doc_id == other.doc_id
    }
}

impl Eq for ScoredDoc {}

impl PartialOrd for ScoredDoc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredDoc {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| self.doc_id.cmp(&other.doc_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sparse_vector_sort_merge_dedup() {
        // (3,1.0),(1,2.0),(3,0.5) -> sorted: 1->2.0, 3->1.5
        let sv = SparseVector::new(vec![(3, 1.0), (1, 2.0), (3, 0.5)]);
        assert_eq!(sv.indices, vec![1, 3]);
        assert_eq!(sv.values, vec![2.0, 1.5]);
    }

    #[test]
    fn test_sparse_vector_zero_filtered() {
        // (1,0.0),(2,1.0) -> only 2->1.0
        let sv = SparseVector::new(vec![(1, 0.0), (2, 1.0)]);
        assert_eq!(sv.indices, vec![2]);
        assert_eq!(sv.values, vec![1.0]);
    }

    #[test]
    fn test_sparse_vector_negatives_allowed() {
        let sv = SparseVector::new(vec![(5, -0.3), (2, 1.0)]);
        assert_eq!(sv.indices, vec![2, 5]);
        assert_eq!(sv.values, vec![1.0, -0.3]);
    }

    #[test]
    fn test_sparse_vector_empty_input() {
        let sv = SparseVector::new(vec![]);
        assert!(sv.is_empty());
        assert_eq!(sv.nnz(), 0);
    }

    #[test]
    fn test_sparse_vector_from_sorted_unchecked() {
        let sv = SparseVector::from_sorted_unchecked(vec![1, 3, 5], vec![0.5, 1.0, 2.0]);
        assert_eq!(sv.indices, vec![1, 3, 5]);
        assert_eq!(sv.values, vec![0.5, 1.0, 2.0]);
    }

    #[test]
    fn test_sparse_vector_nnz() {
        let sv = SparseVector::new(vec![(1, 1.0), (2, 2.0), (3, 3.0)]);
        assert_eq!(sv.nnz(), 3);
    }

    #[test]
    fn test_sparse_vector_dot_product() {
        let a = SparseVector::new(vec![(1, 2.0), (3, 1.0)]);
        let b = SparseVector::new(vec![(1, 0.5), (2, 1.0), (3, 3.0)]);
        let result = a.dot(&b);
        assert!((result - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_sparse_vector_dot_disjoint() {
        let a = SparseVector::new(vec![(1, 1.0), (2, 2.0)]);
        let b = SparseVector::new(vec![(3, 3.0), (4, 4.0)]);
        assert!((a.dot(&b)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_sparse_vector_dot_empty() {
        let a = SparseVector::new(vec![(1, 1.0)]);
        let b = SparseVector::new(vec![]);
        assert!((a.dot(&b)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_posting_entry_size() {
        // PostingEntry is #[repr(C)] with u64 (8 bytes) + f32 (4 bytes) + 4 bytes alignment
        // padding = 16 bytes total. The on-disk packed layout uses 12 bytes (POSTING_DISK_SIZE).
        assert_eq!(std::mem::size_of::<PostingEntry>(), 16);
    }

    #[test]
    fn test_scored_doc_ordering() {
        let high = ScoredDoc {
            score: 5.0,
            doc_id: 1,
        };
        let low = ScoredDoc {
            score: 2.0,
            doc_id: 2,
        };
        assert!(high > low);
    }

    #[test]
    fn test_scored_doc_tiebreak_by_doc_id() {
        let a = ScoredDoc {
            score: 3.0,
            doc_id: 1,
        };
        let b = ScoredDoc {
            score: 3.0,
            doc_id: 2,
        };
        assert!(a < b); // Same score, lower doc_id is "less"
    }

    #[test]
    fn test_sparse_vector_merge_cancellation() {
        // Duplicate indices that sum to zero should be filtered
        let sv = SparseVector::new(vec![(1, 1.0), (1, -1.0), (2, 3.0)]);
        assert_eq!(sv.indices, vec![2]);
        assert_eq!(sv.values, vec![3.0]);
    }
}
