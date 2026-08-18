//! Shared result resolution helpers for search methods.
//!
//! Eliminates duplicated point hydration logic (fetching vector + payload
//! from storage and building `SearchResult`) across vector, text, batch,
//! and sparse search modules.
//!
//! These helpers are ready for adoption by search submodules.
//! Currently tested directly; callers will migrate in a follow-up.

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::error::Error;
use crate::point::{Point, SearchResult};
use crate::scored_result::ScoredResult;
use crate::storage::{PayloadStorage, VectorStorage};

/// Hydrates a single `(id, score)` pair into a `SearchResult` by fetching
/// vector and payload from storage.
///
/// Returns `None` if the vector cannot be retrieved (deleted point) or the
/// payload is TTL-expired at `now_secs` (expired points are invisible on
/// every read surface).
#[inline]
pub(crate) fn hydrate_point(
    id: u64,
    score: f32,
    now_secs: u64,
    vector_storage: &dyn VectorStorage,
    payload_storage: &dyn PayloadStorage,
) -> Option<SearchResult> {
    let vector = vector_storage.retrieve(id).ok().flatten()?;
    let payload = payload_storage.retrieve(id).ok().flatten();
    if is_payload_expired(payload.as_ref(), now_secs) {
        return None;
    }
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
    let now_secs = now_unix_secs();
    pairs
        .iter()
        .take(limit)
        .filter_map(|&(id, score)| {
            hydrate_point(id, score, now_secs, vector_storage, payload_storage)
        })
        .collect()
}

/// Resolves `ScoredResult` values into full `SearchResult` with point data.
pub(crate) fn resolve_scored_results(
    results: &[ScoredResult],
    vector_storage: &dyn VectorStorage,
    payload_storage: &dyn PayloadStorage,
) -> Vec<SearchResult> {
    let now_secs = now_unix_secs();
    results
        .iter()
        .filter_map(|sr| hydrate_point(sr.id, sr.score, now_secs, vector_storage, payload_storage))
        .collect()
}

/// Sorts `SearchResult` values by score according to metric direction.
///
/// - `higher_is_better = true`: descending (cosine, dot product)
/// - `higher_is_better = false`: ascending (euclidean distance)
///
/// Uses unstable sort: equal-score tie-breaking order is irrelevant for ranking.
pub(crate) fn sort_results_by_metric(results: &mut [SearchResult], higher_is_better: bool) {
    results.sort_unstable_by(|a, b| {
        if higher_is_better {
            // Descending: NaN treated as worst (placed at the end).
            // partial_cmp returns None for NaN; the unwrap_or arms put any NaN
            // result after all finite scores, preserving result quality when a
            // score is accidentally NaN (e.g. zero-norm vector in SIMD path).
            b.score.partial_cmp(&a.score).unwrap_or_else(|| {
                match (a.score.is_nan(), b.score.is_nan()) {
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                }
            })
        } else {
            // Ascending (lower distance = better): total_cmp gives a true total
            // order; NaN sorts after +∞, so NaN distances end up last (worst).
            a.score.total_cmp(&b.score)
        }
    });
}

/// Sorts `ScoredResult` values by score according to metric direction.
///
/// Uses unstable sort: equal-score tie-breaking order is irrelevant for ranking.
pub(crate) fn sort_scored_by_metric(results: &mut [ScoredResult], higher_is_better: bool) {
    results.sort_unstable_by(|a, b| {
        if higher_is_better {
            b.score.partial_cmp(&a.score).unwrap_or_else(|| {
                match (a.score.is_nan(), b.score.is_nan()) {
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                }
            })
        } else {
            a.score.total_cmp(&b.score)
        }
    });
}

/// Sorts `SearchResult` values by score descending (higher scores first).
///
/// Used for BM25 text search, sparse search, and fusion results where
/// higher scores always indicate better matches.
///
/// Uses unstable sort: equal-score tie-breaking order is irrelevant for ranking.
#[allow(dead_code)] // Reason: BM25/sparse search utility — callers exist in test suite; future SDK wiring pending
pub(crate) fn sort_results_descending(results: &mut [SearchResult]) {
    results.sort_unstable_by(|a, b| {
        b.score.partial_cmp(&a.score).unwrap_or_else(|| {
            match (a.score.is_nan(), b.score.is_nan()) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Equal,
            }
        })
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

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
