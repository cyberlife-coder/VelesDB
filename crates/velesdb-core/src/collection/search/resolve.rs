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

/// Hydrates, in `pairs` order, the first `limit` points that exist, are not
/// TTL-expired at `now_secs` and whose payload `accept` takes (`None` when
/// the point has none).
///
/// A point's payload is read and judged before its vector is copied, so a
/// filtered search copies the vectors of the `limit` points it returns, not
/// of every candidate it rejects.
pub(crate) fn hydrate_accepted(
    pairs: impl IntoIterator<Item = (u64, f32)>,
    limit: usize,
    now_secs: u64,
    vector_storage: &dyn VectorStorage,
    payload_storage: &dyn PayloadStorage,
    accept: impl Fn(Option<&serde_json::Value>) -> bool,
) -> Vec<SearchResult> {
    let mut results = Vec::new();
    for (id, score) in pairs {
        if results.len() >= limit {
            break;
        }
        let payload = payload_storage.retrieve(id).ok().flatten();
        if is_payload_expired(payload.as_ref(), now_secs) || !accept(payload.as_ref()) {
            continue;
        }
        let Some(vector) = vector_storage.retrieve(id).ok().flatten() else {
            continue;
        };
        let point = Point {
            id,
            vector,
            payload,
            sparse_vectors: None,
        };
        results.push(SearchResult::new(point, score));
    }
    results
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

/// Sorts `(id, score, payload)` candidates by score according to metric
/// direction: descending when `higher_is_better` (cosine, dot product),
/// ascending otherwise (euclidean distance). The deferred-hydration filter
/// path ranks candidates before any vector is read, so it sorts triples, not
/// results.
pub(crate) fn sort_scored_ids_by_metric<P>(entries: &mut [(u64, f32, P)], higher_is_better: bool) {
    entries.sort_unstable_by(|a, b| metric_score_order(a.1, b.1, higher_is_better));
}

/// Shared score ordering for the metric direction.
///
/// - `higher_is_better = true`: descending; NaN treated as worst (placed at
///   the end). `partial_cmp` returns `None` for NaN; the fallback arms put
///   any NaN result after all finite scores, preserving result quality when
///   a score is accidentally NaN (e.g. zero-norm vector in SIMD path).
/// - `higher_is_better = false`: ascending (lower distance = better);
///   `total_cmp` gives a true total order and NaN sorts after +inf, so NaN
///   distances end up last (worst).
fn metric_score_order(a: f32, b: f32, higher_is_better: bool) -> std::cmp::Ordering {
    if higher_is_better {
        b.partial_cmp(&a)
            .unwrap_or_else(|| match (a.is_nan(), b.is_nan()) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Equal,
            })
    } else {
        a.total_cmp(&b)
    }
}

/// Sorts `ScoredResult` values by score according to metric direction.
///
/// Uses unstable sort: equal-score tie-breaking order is irrelevant for ranking.
pub(crate) fn sort_scored_by_metric(results: &mut [ScoredResult], higher_is_better: bool) {
    results.sort_unstable_by(|a, b| metric_score_order(a.score, b.score, higher_is_better));
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
