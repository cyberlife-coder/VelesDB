//! Merge logic for combining HNSW search results with delta buffer results.
//!
//! Extracted from `delta.rs` to keep that module focused on the `DeltaBuffer`
//! state machine and its core operations.

use rustc_hash::FxHashSet;

use crate::distance::DistanceMetric;

use super::delta::DeltaBuffer;

/// Merges HNSW search results with delta buffer brute-force results.
///
/// If the delta buffer is not active (or draining), returns `hnsw_results`
/// unchanged. Otherwise, performs a brute-force scan of the delta, deduplicates
/// by ID (delta entries win on conflict since they may be more recent), sorts by
/// the metric's ordering, and truncates to `k`.
#[must_use]
pub fn merge_with_delta(
    hnsw_results: Vec<(u64, f32)>,
    delta: &DeltaBuffer,
    query: &[f32],
    k: usize,
    metric: DistanceMetric,
) -> Vec<(u64, f32)> {
    if !delta.is_searchable() {
        return hnsw_results;
    }

    let delta_results = delta.search(query, k, metric);
    if delta_results.is_empty() {
        return hnsw_results;
    }

    // Delta IDs win on duplicates (more recent data).
    // FxHashSet: this merge sits on the unconditional search path and the
    // std SipHash set cost showed in the audit; ids are already random-ish.
    let delta_ids: FxHashSet<u64> = delta_results.iter().map(|(id, _)| *id).collect();
    let mut merged: Vec<(u64, f32)> = hnsw_results
        .into_iter()
        .filter(|(id, _)| !delta_ids.contains(id))
        .collect();
    merged.extend(delta_results);

    metric.sort_results(&mut merged);
    merged.truncate(k);
    merged
}

/// Merges HNSW search results (as [`crate::ScoredResult`]) with delta buffer
/// results.
///
/// Takes and returns [`crate::ScoredResult`] so callers already on that shape
/// need no conversion pass of their own around the merge.
///
/// It is NOT allocation-free, and the doc claimed it was: the body collects
/// into a `Vec<(u64, f32)>` and collects back, which is the very round-trip the
/// old wording said it avoided. Merging on tuples is deliberate — it shares
/// [`DistanceMetric::sort_results`] and the truncate with [`merge_with_delta`],
/// and duplicating both to drop two allocations is a trade nobody has measured.
/// Measure before making that trade; do not restore the claim without it.
#[must_use]
pub fn merge_with_delta_scored(
    hnsw_results: Vec<crate::scored_result::ScoredResult>,
    delta: &DeltaBuffer,
    query: &[f32],
    k: usize,
    metric: DistanceMetric,
) -> Vec<crate::scored_result::ScoredResult> {
    if !delta.is_searchable() {
        return hnsw_results;
    }

    merge_pending_scored(hnsw_results, delta.search(query, k, metric), k, metric)
}

/// Merges index results with the scores of points not indexed yet — the
/// delta buffer's, or the async index builder's queues' — keeping the top `k`
/// by the metric's ordering.
///
/// A pending score replaces an index result for the same id, so each id is
/// returned once: the delta buffer holds the more recent write, and an id the
/// builder still queues may already be linked by a racing flush or a vacuum.
#[must_use]
pub(crate) fn merge_pending_scored(
    hnsw_results: Vec<crate::scored_result::ScoredResult>,
    pending: Vec<(u64, f32)>,
    k: usize,
    metric: DistanceMetric,
) -> Vec<crate::scored_result::ScoredResult> {
    if pending.is_empty() {
        return hnsw_results;
    }

    // FxHashSet: this merge sits on the unconditional search path and the
    // std SipHash set cost showed in the audit; ids are already random-ish.
    let pending_ids: FxHashSet<u64> = pending.iter().map(|(id, _)| *id).collect();
    let mut merged: Vec<(u64, f32)> = hnsw_results
        .into_iter()
        .filter(|sr| !pending_ids.contains(&sr.id))
        .map(Into::into)
        .collect();
    merged.extend(pending);

    metric.sort_results(&mut merged);
    merged.truncate(k);
    merged
        .into_iter()
        .map(crate::scored_result::ScoredResult::from)
        .collect()
}
