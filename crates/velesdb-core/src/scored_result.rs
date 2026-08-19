//! Unified scored result type for vector search and graph traversal.
//!
//! `ScoredResult` replaces scattered `(u64, f32)` tuple patterns across search
//! paths, providing a named, self-documenting type with bidirectional conversions.

use crate::sparse_index::types::ScoredDoc;

/// A search result pairing an item identifier with a relevance score.
///
/// Used as the canonical return type across vector search, sparse search,
/// and hybrid fusion pipelines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoredResult {
    /// Unique identifier of the matched item.
    pub id: u64,
    /// Relevance score (interpretation depends on metric: higher may be better
    /// for similarity, lower for distance).
    pub score: f32,
}

impl ScoredResult {
    /// Creates a new scored result.
    #[must_use]
    #[inline]
    pub fn new(id: u64, score: f32) -> Self {
        Self { id, score }
    }
}

// --- Tuple conversions ---

impl From<(u64, f32)> for ScoredResult {
    #[inline]
    fn from((id, score): (u64, f32)) -> Self {
        Self { id, score }
    }
}

impl From<ScoredResult> for (u64, f32) {
    #[inline]
    fn from(sr: ScoredResult) -> Self {
        (sr.id, sr.score)
    }
}

// --- ScoredDoc conversions ---

impl From<ScoredDoc> for ScoredResult {
    #[inline]
    fn from(sd: ScoredDoc) -> Self {
        Self {
            id: sd.doc_id,
            score: sd.score,
        }
    }
}

impl From<ScoredResult> for ScoredDoc {
    #[inline]
    fn from(sr: ScoredResult) -> Self {
        Self {
            doc_id: sr.id,
            score: sr.score,
        }
    }
}

#[cfg(test)]
#[path = "scored_result_tests.rs"]
mod tests;
