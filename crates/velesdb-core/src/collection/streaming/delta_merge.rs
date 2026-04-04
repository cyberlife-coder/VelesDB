//! Merge logic for combining HNSW search results with delta buffer results.
//!
//! Extracted from `delta.rs` to keep that module focused on the `DeltaBuffer`
//! state machine and its core operations.

use std::collections::HashSet;

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
    let delta_ids: HashSet<u64> = delta_results.iter().map(|(id, _)| *id).collect();
    let mut merged: Vec<(u64, f32)> = hnsw_results
        .into_iter()
        .filter(|(id, _)| !delta_ids.contains(id))
        .collect();
    merged.extend(delta_results);

    metric.sort_results(&mut merged);
    merged.truncate(k);
    merged
}

/// Merges HNSW search results (as [`ScoredResult`]) with delta buffer results.
///
/// Zero-allocation variant that avoids the `ScoredResult` → `(u64, f32)` →
/// `ScoredResult` round-trip in the search pipeline.
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

    let delta_results = delta.search(query, k, metric);
    if delta_results.is_empty() {
        return hnsw_results;
    }

    let delta_ids: HashSet<u64> = delta_results.iter().map(|(id, _)| *id).collect();
    let mut merged: Vec<(u64, f32)> = hnsw_results
        .into_iter()
        .filter(|sr| !delta_ids.contains(&sr.id))
        .map(Into::into)
        .collect();
    merged.extend(delta_results);

    metric.sort_results(&mut merged);
    merged.truncate(k);
    merged
        .into_iter()
        .map(crate::scored_result::ScoredResult::from)
        .collect()
}
