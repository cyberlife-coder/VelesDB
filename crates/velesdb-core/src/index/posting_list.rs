//! Adaptive posting list for BM25 inverted index.
#![allow(clippy::doc_markdown)]
#![allow(clippy::default_trait_access)]
#![allow(clippy::cast_possible_truncation)]
//!
//! This module provides a hybrid posting list that automatically switches between:
//! - `FxHashSet<u32>` for low-cardinality terms (< 1000 docs)
//! - `RoaringBitmap` for high-cardinality terms (≥ 1000 docs)
//!
//! # Performance Characteristics
//!
//! | Operation | Small (HashSet) | Large (Roaring) |
//! |-----------|-----------------|-----------------|
//! | Insert    | O(1) amortized  | O(log n)        |
//! | Contains  | O(1)            | O(log n)        |
//! | Union     | O(n + m)        | O(min(n,m))     |
//! | Memory    | ~24 bytes/doc   | ~2-4 bytes/doc  |
//!
//! The crossover point (~1000 docs) is chosen based on benchmarks showing
//! Roaring overhead is amortized at this cardinality.

use roaring::RoaringBitmap;
use rustc_hash::FxHashSet;

/// Threshold for switching from Small to Large representation.
/// Benchmarks show Roaring overhead is amortized above this cardinality.
pub const PROMOTION_THRESHOLD: usize = 1000;

/// Adaptive posting list that chooses optimal representation based on cardinality.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Some methods used only in tests or for future API
pub enum PostingList {
    /// Small posting list using FxHashSet (fast for < 1000 docs)
    Small(FxHashSet<u32>),
    /// Large posting list using RoaringBitmap (efficient for ≥ 1000 docs)
    Large(RoaringBitmap),
}

impl Default for PostingList {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // Some methods used only in tests or reserved for future API
impl PostingList {
    /// Creates a new empty posting list (starts as Small).
    #[must_use]
    pub fn new() -> Self {
        PostingList::Small(FxHashSet::default())
    }

    /// Creates a posting list with expected capacity hint.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        if capacity >= PROMOTION_THRESHOLD {
            PostingList::Large(RoaringBitmap::new())
        } else {
            PostingList::Small(FxHashSet::with_capacity_and_hasher(
                capacity,
                Default::default(),
            ))
        }
    }

    /// Inserts a document ID into the posting list.
    ///
    /// Automatically promotes to Large representation if threshold is exceeded.
    pub fn insert(&mut self, doc_id: u32) -> bool {
        match self {
            PostingList::Small(set) => {
                let inserted = set.insert(doc_id);
                // Check if we need to promote to Large
                if set.len() >= PROMOTION_THRESHOLD {
                    self.promote_to_large();
                }
                inserted
            }
            PostingList::Large(bitmap) => bitmap.insert(doc_id),
        }
    }

    /// Removes a document ID from the posting list.
    pub fn remove(&mut self, doc_id: u32) -> bool {
        match self {
            PostingList::Small(set) => set.remove(&doc_id),
            PostingList::Large(bitmap) => bitmap.remove(doc_id),
        }
    }

    /// Checks if the posting list contains a document ID.
    #[must_use]
    pub fn contains(&self, doc_id: u32) -> bool {
        match self {
            PostingList::Small(set) => set.contains(&doc_id),
            PostingList::Large(bitmap) => bitmap.contains(doc_id),
        }
    }

    /// Returns the number of document IDs in the posting list.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            PostingList::Small(set) => set.len(),
            PostingList::Large(bitmap) => bitmap.len() as usize,
        }
    }

    /// Returns true if the posting list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns true if this is the Large (Roaring) representation.
    #[must_use]
    pub fn is_large(&self) -> bool {
        matches!(self, PostingList::Large(_))
    }

    /// Performs union with another posting list, returning a new one.
    ///
    /// Optimized: uses Roaring union when both are Large.
    #[must_use]
    pub fn union(&self, other: &PostingList) -> PostingList {
        match (self, other) {
            // Both Large: use efficient Roaring union
            (PostingList::Large(a), PostingList::Large(b)) => PostingList::Large(a | b),
            // One or both Small: convert and union
            (PostingList::Large(bitmap), PostingList::Small(set))
            | (PostingList::Small(set), PostingList::Large(bitmap)) => {
                let mut result = bitmap.clone();
                for &doc_id in set {
                    result.insert(doc_id);
                }
                PostingList::Large(result)
            }
            // Both Small: check if result needs promotion
            (PostingList::Small(a), PostingList::Small(b)) => {
                let combined_estimate = a.len() + b.len();
                if combined_estimate >= PROMOTION_THRESHOLD {
                    // Promote to Roaring
                    let mut bitmap = RoaringBitmap::new();
                    for &doc_id in a {
                        bitmap.insert(doc_id);
                    }
                    for &doc_id in b {
                        bitmap.insert(doc_id);
                    }
                    PostingList::Large(bitmap)
                } else {
                    let mut result = a.clone();
                    result.extend(b.iter().copied());
                    PostingList::Small(result)
                }
            }
        }
    }

    /// Iterates over all document IDs in the posting list.
    pub fn iter(&self) -> PostingListIter<'_> {
        match self {
            PostingList::Small(set) => PostingListIter::Small(set.iter()),
            PostingList::Large(bitmap) => PostingListIter::Large(bitmap.iter()),
        }
    }

    /// Promotes a Small posting list to Large representation.
    fn promote_to_large(&mut self) {
        if let PostingList::Small(set) = self {
            let mut bitmap = RoaringBitmap::new();
            for &doc_id in set.iter() {
                bitmap.insert(doc_id);
            }
            *self = PostingList::Large(bitmap);
        }
    }
}

/// Iterator over posting list document IDs.
pub enum PostingListIter<'a> {
    Small(std::collections::hash_set::Iter<'a, u32>),
    Large(roaring::bitmap::Iter<'a>),
}

impl Iterator for PostingListIter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PostingListIter::Small(iter) => iter.next().copied(),
            PostingListIter::Large(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            PostingListIter::Small(iter) => iter.size_hint(),
            PostingListIter::Large(iter) => iter.size_hint(),
        }
    }
}

impl ExactSizeIterator for PostingListIter<'_> {}
