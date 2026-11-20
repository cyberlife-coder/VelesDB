//! Internal vector operations for `VectorStore`.
//!
//! This module contains extracted scoring and search logic to reduce lib.rs size.
//! These are internal helpers, not exposed via `wasm_bindgen`.
//!
//! Every storage mode keeps the f32 vectors, so scoring is the exact
//! `DistanceMetric::calculate` over `data` whatever the mode — the same exact
//! f32 scores `velesdb-core` returns (its quantized paths re-rank with f32).

use velesdb_core::DistanceMetric;

/// Compute similarity scores for all vectors against a query.
///
/// Returns `(id, score)` pairs in storage order: none when `query` cannot be
/// compared with the stored vectors (another length, or a store holding
/// none — `DistanceMetric::checked_calculate`), which the callers' query
/// checks refuse first.
pub fn compute_scores(
    query: &[f32],
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: DistanceMetric,
) -> Vec<(u64, f32)> {
    ids.iter()
        .enumerate()
        .filter_map(|(idx, &id)| {
            Some((
                id,
                metric.checked_calculate(query, vector_at(data, idx, dimension))?,
            ))
        })
        .collect()
}

/// Sort results by relevance.
pub fn sort_results(results: &mut [(u64, f32)], higher_is_better: bool) {
    results.sort_by(|a, b| velesdb_core::distance::rank_scores(a.1, b.1, higher_is_better));
}

/// Compute scores with payload filtering.
pub fn compute_filtered_scores<'a, F>(
    query: &[f32],
    ids: &[u64],
    payloads: &'a [Option<serde_json::Value>],
    data: &[f32],
    dimension: usize,
    metric: DistanceMetric,
    predicate: F,
) -> Vec<(u64, f32, Option<&'a serde_json::Value>)>
where
    F: Fn(u64, Option<&serde_json::Value>) -> bool,
{
    // A point without payload is tested with a `null` payload, as in core.
    ids.iter()
        .enumerate()
        .filter_map(|(idx, &id)| {
            let payload = payloads[idx].as_ref();
            if !predicate(id, payload) {
                return None;
            }
            let score = metric.checked_calculate(query, vector_at(data, idx, dimension))?;
            Some((id, score, payload))
        })
        .collect()
}

/// The f32 vector stored at row `idx` of the contiguous `data` buffer.
fn vector_at(data: &[f32], idx: usize, dimension: usize) -> &[f32] {
    let start = idx * dimension;
    &data[start..start + dimension]
}

#[cfg(test)]
#[path = "vector_ops_tests.rs"]
mod tests;
