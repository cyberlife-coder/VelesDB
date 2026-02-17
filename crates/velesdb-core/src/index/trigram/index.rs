//! Trigram Index implementation using Roaring Bitmaps.
//!
//! SOTA 2026 implementation based on:
//! - PostgreSQL pg_trgm algorithm
//! - arXiv:2310.11703v2 recommendations
//! - Roaring Bitmaps for compressed bitmap operations

#![allow(clippy::cast_possible_truncation)] // RoaringBitmap uses u32, truncation is acceptable
#![allow(clippy::cast_precision_loss)] // Precision loss acceptable for scoring

use roaring::RoaringBitmap;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashSet;

/// Trigram type: 3 bytes representing a trigram.
pub type Trigram = [u8; 3];

/// Extract trigrams from text with padding.
///
/// Following `PostgreSQL` `pg_trgm` algorithm:
/// - Pad text with 2 spaces before and after
/// - Extract all 3-byte sequences
///
/// # Example
///
/// ```
/// use velesdb_core::index::trigram::extract_trigrams;
///
/// let trigrams = extract_trigrams("hello");
/// // "  hello  " → {"  h", " he", "hel", "ell", "llo", "lo ", "o  "}
/// assert_eq!(trigrams.len(), 7);
/// ```
#[must_use]
pub fn extract_trigrams(text: &str) -> HashSet<Trigram> {
    extract_trigrams_internal(text, true)
}

/// Extract trigrams for pattern matching (no trailing padding).
///
/// For LIKE pattern matching, we don't want trailing padding
/// because the pattern is a substring, not a complete match.
#[must_use]
pub fn extract_trigrams_for_pattern(text: &str) -> HashSet<Trigram> {
    extract_trigrams_internal(text, false)
}

fn extract_trigrams_internal(text: &str, trailing_padding: bool) -> HashSet<Trigram> {
    if text.is_empty() {
        return HashSet::new();
    }

    let text_bytes = text.as_bytes();
    let text_len = text_bytes.len();

    // Pre-calculate capacity to avoid reallocations
    let trailing_pad = if trailing_padding { 2 } else { 0 };
    let total_len = 2 + text_len + trailing_pad;
    let trigram_count = if total_len >= 3 { total_len - 2 } else { 0 };

    let mut trigrams = HashSet::with_capacity(trigram_count);

    // Zero-copy extraction: handle padding virtually without format!
    // Conceptual string: "  " + text + ("  " if trailing_padding else "")
    // We extract trigrams by computing indices into this virtual string

    for i in 0..trigram_count {
        let trigram: [u8; 3] = std::array::from_fn(|j| {
            let pos = i + j;
            if pos < 2 {
                b' ' // Leading padding
            } else if pos < 2 + text_len {
                text_bytes[pos - 2]
            } else {
                b' ' // Trailing padding
            }
        });
        trigrams.insert(trigram);
    }

    trigrams
}

/// Statistics for the trigram index.
#[derive(Debug, Clone, Default)]
pub struct TrigramStats {
    /// Number of indexed documents.
    pub doc_count: u64,
    /// Number of unique trigrams.
    pub trigram_count: usize,
    /// Estimated memory usage in bytes.
    pub memory_bytes: usize,
}

/// Trigram-based inverted index using Roaring Bitmaps.
///
/// Provides O(1) trigram lookup and O(k) intersection for k trigrams.
/// Memory-efficient through Roaring Bitmap compression.
#[derive(Debug, Default)]
pub struct TrigramIndex {
    /// Inverted index: trigram → bitmap of doc IDs containing it.
    inverted: FxHashMap<Trigram, RoaringBitmap>,

    /// Document trigrams: `doc_id` → set of trigrams (for removal and scoring).
    doc_trigrams: FxHashMap<u64, FxHashSet<Trigram>>,

    /// All document IDs (for empty pattern queries).
    all_docs: RoaringBitmap,
}

impl TrigramIndex {
    /// Create a new empty trigram index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.all_docs.is_empty()
    }

    /// Get the number of indexed documents.
    #[must_use]
    pub fn doc_count(&self) -> u64 {
        self.all_docs.len()
    }

    /// Insert a document into the index.
    ///
    /// If a document with the same ID exists, it will be updated.
    ///
    /// # Panics
    ///
    /// Panics if `doc_id` exceeds `u32::MAX` (4 billion documents).
    /// This is a limitation of the underlying RoaringBitmap storage.
    pub fn insert(&mut self, doc_id: u64, text: &str) {
        // Bounds check: RoaringBitmap uses u32 internally
        assert!(
            u32::try_from(doc_id).is_ok(),
            "TrigramIndex: doc_id {doc_id} exceeds u32::MAX limit. Maximum 4B documents supported."
        );

        // Remove old entry if exists
        if self.doc_trigrams.contains_key(&doc_id) {
            self.remove(doc_id);
        }

        let trigrams = extract_trigrams(text);

        // Store trigrams for this document
        let trigram_set: FxHashSet<Trigram> = trigrams.iter().copied().collect();
        self.doc_trigrams.insert(doc_id, trigram_set);

        // Add to inverted index
        // SAFETY: Bounds checked above, truncation is safe
        #[allow(clippy::cast_possible_truncation)]
        let doc_id_u32 = doc_id as u32;
        for trigram in trigrams {
            self.inverted.entry(trigram).or_default().insert(doc_id_u32);
        }

        // Track document
        self.all_docs.insert(doc_id_u32);
    }

    /// Remove a document from the index.
    ///
    /// # Note
    ///
    /// If `doc_id` exceeds `u32::MAX`, the document won't exist in the index anyway.
    pub fn remove(&mut self, doc_id: u64) {
        // If doc_id > u32::MAX, it was never inserted (bounds checked in insert)
        if u32::try_from(doc_id).is_err() {
            return;
        }
        // SAFETY: Bounds checked above
        #[allow(clippy::cast_possible_truncation)]
        let doc_id_u32 = doc_id as u32;
        if let Some(trigrams) = self.doc_trigrams.remove(&doc_id) {
            // Remove from inverted index
            for trigram in trigrams {
                if let Some(bitmap) = self.inverted.get_mut(&trigram) {
                    bitmap.remove(doc_id_u32);
                    // Clean up empty bitmaps
                    if bitmap.is_empty() {
                        self.inverted.remove(&trigram);
                    }
                }
            }
        }

        self.all_docs.remove(doc_id_u32);
    }

    /// Search for documents matching a LIKE pattern.
    ///
    /// Returns a bitmap of document IDs that potentially match.
    /// This is a candidate filter - results should be verified with actual LIKE matching.
    ///
    /// # Algorithm
    ///
    /// 1. Extract trigrams from pattern (without trailing padding)
    /// 2. Intersect bitmaps for all trigrams
    /// 3. Return candidate set
    #[must_use]
    pub fn search_like(&self, pattern: &str) -> RoaringBitmap {
        // Empty pattern matches all documents
        if pattern.is_empty() {
            return self.all_docs.clone();
        }

        // Use pattern extraction (no trailing padding) for substring matching
        let trigrams = extract_trigrams_for_pattern(pattern);

        // Short patterns (< 3 chars after padding) - use prefix matching
        if trigrams.is_empty() {
            return self.all_docs.clone();
        }

        // Intersect bitmaps for all query trigrams
        let mut result: Option<RoaringBitmap> = None;

        for trigram in &trigrams {
            match self.inverted.get(trigram) {
                Some(bitmap) => {
                    result = Some(match result {
                        Some(acc) => acc & bitmap,
                        None => bitmap.clone(),
                    });
                }
                None => {
                    // Trigram not found - no documents match
                    return RoaringBitmap::new();
                }
            }
        }

        result.unwrap_or_default()
    }

    /// Calculate Jaccard similarity score for a document against query trigrams.
    ///
    /// Score = |intersection| / |union|
    ///
    /// Returns a value between 0.0 (no overlap) and 1.0 (identical).
    #[must_use]
    pub fn score_jaccard(&self, doc_id: u64, query_trigrams: &HashSet<Trigram>) -> f32 {
        let Some(doc_trigrams) = self.doc_trigrams.get(&doc_id) else {
            return 0.0;
        };

        if doc_trigrams.is_empty() || query_trigrams.is_empty() {
            return 0.0;
        }

        // Convert to HashSet for intersection/union
        let doc_set: HashSet<&Trigram> = doc_trigrams.iter().collect();
        let query_set: HashSet<&Trigram> = query_trigrams.iter().collect();

        let intersection = doc_set.intersection(&query_set).count();
        let union = doc_set.union(&query_set).count();

        if union == 0 {
            0.0
        } else {
            intersection as f32 / union as f32
        }
    }

    /// Search for documents matching a LIKE pattern with threshold pruning.
    ///
    /// Returns a vector of (`doc_id`, score) tuples sorted by score descending.
    /// Documents with score below threshold are filtered out.
    ///
    /// # Arguments
    ///
    /// * `pattern` - The search pattern
    /// * `threshold` - Minimum Jaccard score (0.0 to 1.0) to include in results
    ///
    /// # Performance
    ///
    /// Threshold pruning reduces result set size and post-processing overhead.
    #[must_use]
    pub fn search_like_ranked(&self, pattern: &str, threshold: f32) -> Vec<(u64, f32)> {
        // Empty pattern returns all docs with score 0
        if pattern.is_empty() {
            return self
                .all_docs
                .iter()
                .map(|id| (u64::from(id), 0.0f32))
                .collect();
        }

        // Get candidates from bitmap intersection
        let candidates = self.search_like(pattern);

        if candidates.is_empty() {
            return Vec::new();
        }

        // Extract query trigrams for scoring
        let query_trigrams = extract_trigrams_for_pattern(pattern);

        // Score and filter candidates
        let mut results: Vec<(u64, f32)> = candidates
            .iter()
            .map(|id| {
                let doc_id = u64::from(id);
                let score = self.score_jaccard(doc_id, &query_trigrams);
                (doc_id, score)
            })
            .filter(|(_, score)| *score >= threshold)
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| b.1.total_cmp(&a.1));

        results
    }

    /// Get index statistics.
    #[must_use]
    pub fn stats(&self) -> TrigramStats {
        // Estimate memory usage
        let inverted_size = self.inverted.len() * (3 + 8); // trigram + pointer
        let bitmap_size: usize = self
            .inverted
            .values()
            .map(roaring::RoaringBitmap::serialized_size)
            .sum();
        let doc_trigrams_size = self.doc_trigrams.len() * 64; // rough estimate

        TrigramStats {
            doc_count: self.all_docs.len(),
            trigram_count: self.inverted.len(),
            memory_bytes: inverted_size + bitmap_size + doc_trigrams_size,
        }
    }
}
