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
/// Returns `(id, score)` pairs in storage order.
pub fn compute_scores(
    query: &[f32],
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: DistanceMetric,
) -> Vec<(u64, f32)> {
    ids.iter()
        .enumerate()
        .map(|(idx, &id)| (id, metric.calculate(query, vector_at(data, idx, dimension))))
        .collect()
}

/// Sort results by relevance.
pub fn sort_results(results: &mut [(u64, f32)], higher_is_better: bool) {
    if higher_is_better {
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    } else {
        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    }
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
    F: Fn(u64, &serde_json::Value) -> bool,
{
    ids.iter()
        .enumerate()
        .filter_map(|(idx, &id)| {
            let payload = payloads[idx].as_ref()?;
            if !predicate(id, payload) {
                return None;
            }
            let score = metric.calculate(query, vector_at(data, idx, dimension));
            Some((id, score, Some(payload)))
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
