//! Shared result resolution helpers for search methods.
//!
//! Eliminates duplicated point hydration logic (fetching vector + payload
//! from storage and building `SearchResult`) across vector, text, batch,
//! and sparse search modules.

use crate::error::Error;
use crate::point::{Point, SearchResult};
use crate::scored_result::ScoredResult;
use crate::storage::{PayloadStorage, VectorStorage};

/// Hydrates a single `(id, score)` pair into a `SearchResult` by fetching
/// vector and payload from storage.
///
/// Returns `None` if the vector cannot be retrieved (deleted point).
#[inline]
pub(crate) fn hydrate_point(
    id: u64,
    score: f32,
    vector_storage: &dyn VectorStorage,
    payload_storage: &dyn PayloadStorage,
) -> Option<SearchResult> {
    let vector = vector_storage.retrieve(id).ok().flatten()?;
    let payload = payload_storage.retrieve(id).ok().flatten();
    let point = Point {
        id,
        vector,
        payload,
        sparse_vectors: None,
    };
    Some(SearchResult::new(point, score))
}

/// Resolves a slice of `(id, score)` tuples into `SearchResult` values,
/// taking at most `limit` results.
pub(crate) fn resolve_id_score_pairs(
    pairs: &[(u64, f32)],
    limit: usize,
    vector_storage: &dyn VectorStorage,
    payload_storage: &dyn PayloadStorage,
) -> Vec<SearchResult> {
    pairs
        .iter()
        .take(limit)
        .filter_map(|&(id, score)| hydrate_point(id, score, vector_storage, payload_storage))
        .collect()
}

/// Resolves `ScoredResult` values into full `SearchResult` with point data.
pub(crate) fn resolve_scored_results(
    results: &[ScoredResult],
    vector_storage: &dyn VectorStorage,
    payload_storage: &dyn PayloadStorage,
) -> Vec<SearchResult> {
    results
        .iter()
        .filter_map(|sr| hydrate_point(sr.id, sr.score, vector_storage, payload_storage))
        .collect()
}

/// Sorts `SearchResult` values by score according to metric direction.
///
/// - `higher_is_better = true`: descending (cosine, dot product)
/// - `higher_is_better = false`: ascending (euclidean distance)
pub(crate) fn sort_results_by_metric(results: &mut [SearchResult], higher_is_better: bool) {
    results.sort_by(|a, b| {
        if higher_is_better {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });
}

/// Sorts `ScoredResult` values by score according to metric direction.
pub(crate) fn sort_scored_by_metric(results: &mut [ScoredResult], higher_is_better: bool) {
    results.sort_by(|a, b| {
        if higher_is_better {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });
}

/// Sorts `SearchResult` values by score descending (higher scores first).
///
/// Used for BM25 text search, sparse search, and fusion results where
/// higher scores always indicate better matches.
#[allow(dead_code)]
pub(crate) fn sort_results_descending(results: &mut [SearchResult]) {
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Creates a "sparse index not found" error with consistent formatting.
///
/// Displays `<default>` for empty index names to aid debugging.
pub(crate) fn sparse_index_not_found(index_name: &str) -> Error {
    Error::Config(format!(
        "Sparse index '{}' not found",
        if index_name.is_empty() {
            "<default>"
        } else {
            index_name
        }
    ))
}
