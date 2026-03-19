//! Shared result resolution helpers for search methods.
//!
//! Eliminates duplicated point hydration logic (fetching vector + payload
//! from storage and building `SearchResult`) across vector, text, batch,
//! and sparse search modules.
//!
//! These helpers are ready for adoption by search submodules.
//! Currently tested directly; callers will migrate in a follow-up.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::Point;

    /// Helper to build a `SearchResult` with a given score and dummy point data.
    fn make_search_result(id: u64, score: f32) -> SearchResult {
        let point = Point::without_payload(id, vec![0.0; 4]);
        SearchResult::new(point, score)
    }

    // --- sort_results_by_metric ---

    #[test]
    fn sort_results_by_metric_descending_when_higher_is_better() {
        let mut results = vec![
            make_search_result(1, 0.3),
            make_search_result(2, 0.9),
            make_search_result(3, 0.6),
        ];
        sort_results_by_metric(&mut results, true);

        assert_eq!(results[0].point.id, 2);
        assert_eq!(results[1].point.id, 3);
        assert_eq!(results[2].point.id, 1);
    }

    #[test]
    fn sort_results_by_metric_ascending_when_lower_is_better() {
        let mut results = vec![
            make_search_result(1, 0.3),
            make_search_result(2, 0.9),
            make_search_result(3, 0.6),
        ];
        sort_results_by_metric(&mut results, false);

        assert_eq!(results[0].point.id, 1);
        assert_eq!(results[1].point.id, 3);
        assert_eq!(results[2].point.id, 2);
    }

    #[test]
    fn sort_results_by_metric_empty_slice_is_noop() {
        let mut results: Vec<SearchResult> = vec![];
        sort_results_by_metric(&mut results, true);
        assert!(results.is_empty());
    }

    #[test]
    fn sort_results_by_metric_single_element() {
        let mut results = vec![make_search_result(1, 0.5)];
        sort_results_by_metric(&mut results, true);
        assert_eq!(results[0].point.id, 1);
    }

    // --- sort_results_descending ---

    #[test]
    fn sort_results_descending_always_highest_first() {
        let mut results = vec![
            make_search_result(10, 0.1),
            make_search_result(20, 0.8),
            make_search_result(30, 0.5),
            make_search_result(40, 1.0),
        ];
        sort_results_descending(&mut results);

        assert_eq!(results[0].point.id, 40);
        assert_eq!(results[1].point.id, 20);
        assert_eq!(results[2].point.id, 30);
        assert_eq!(results[3].point.id, 10);
    }

    #[test]
    fn sort_results_descending_equal_scores_stable_enough() {
        let mut results = vec![make_search_result(1, 0.5), make_search_result(2, 0.5)];
        sort_results_descending(&mut results);
        // Both have the same score; we just verify no panic and both are present.
        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
    }

    // --- sort_scored_by_metric ---

    #[test]
    fn sort_scored_by_metric_descending_when_higher_is_better() {
        let mut results = vec![
            ScoredResult::new(1, 0.2),
            ScoredResult::new(2, 0.8),
            ScoredResult::new(3, 0.5),
        ];
        sort_scored_by_metric(&mut results, true);

        assert_eq!(results[0].id, 2);
        assert_eq!(results[1].id, 3);
        assert_eq!(results[2].id, 1);
    }

    #[test]
    fn sort_scored_by_metric_ascending_when_lower_is_better() {
        let mut results = vec![
            ScoredResult::new(1, 0.2),
            ScoredResult::new(2, 0.8),
            ScoredResult::new(3, 0.5),
        ];
        sort_scored_by_metric(&mut results, false);

        assert_eq!(results[0].id, 1);
        assert_eq!(results[1].id, 3);
        assert_eq!(results[2].id, 2);
    }

    #[test]
    fn sort_scored_by_metric_empty_slice_is_noop() {
        let mut results: Vec<ScoredResult> = vec![];
        sort_scored_by_metric(&mut results, true);
        assert!(results.is_empty());
    }

    // --- sparse_index_not_found ---

    #[test]
    fn sparse_index_not_found_with_named_index() {
        let err = sparse_index_not_found("my_index");
        let msg = err.to_string();
        assert!(
            msg.contains("my_index"),
            "Error message should contain the index name, got: {msg}"
        );
        // Verify it is an Error::Config variant.
        assert!(matches!(err, Error::Config(_)));
    }

    #[test]
    fn sparse_index_not_found_empty_name_shows_default() {
        let err = sparse_index_not_found("");
        let msg = err.to_string();
        assert!(
            msg.contains("<default>"),
            "Empty name should display <default>, got: {msg}"
        );
        assert!(matches!(err, Error::Config(_)));
    }

    #[test]
    fn sparse_index_not_found_nonempty_does_not_show_default() {
        let err = sparse_index_not_found("title_sparse");
        let msg = err.to_string();
        assert!(
            !msg.contains("<default>"),
            "Non-empty name should not show <default>, got: {msg}"
        );
    }
}
