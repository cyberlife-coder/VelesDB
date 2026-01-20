//! Internal vector operations for VectorStore.
//!
//! This module contains extracted scoring and search logic to reduce lib.rs size.
//! These are internal helpers, not exposed via wasm_bindgen.

use crate::distance::DistanceMetric;
use crate::StorageMode;

/// Compute similarity scores for all vectors against a query.
///
/// Handles all storage modes (Full, SQ8, Binary) and returns (id, score) pairs.
#[allow(clippy::too_many_arguments)]
pub fn compute_scores(
    query: &[f32],
    ids: &[u64],
    data: &[f32],
    data_sq8: &[u8],
    data_binary: &[u8],
    sq8_mins: &[f32],
    sq8_scales: &[f32],
    dimension: usize,
    metric: &DistanceMetric,
    storage_mode: StorageMode,
) -> Vec<(u64, f32)> {
    match storage_mode {
        StorageMode::Full => compute_scores_full(query, ids, data, dimension, metric),
        StorageMode::SQ8 => compute_scores_sq8(
            query, ids, data_sq8, sq8_mins, sq8_scales, dimension, metric,
        ),
        StorageMode::Binary => compute_scores_binary(query, ids, data_binary, dimension, metric),
    }
}

/// Compute scores for Full precision storage.
fn compute_scores_full(
    query: &[f32],
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: &DistanceMetric,
) -> Vec<(u64, f32)> {
    ids.iter()
        .enumerate()
        .map(|(idx, &id)| {
            let start = idx * dimension;
            let v_data = &data[start..start + dimension];
            let score = metric.calculate(query, v_data);
            (id, score)
        })
        .collect()
}

/// Compute scores for SQ8 quantized storage.
fn compute_scores_sq8(
    query: &[f32],
    ids: &[u64],
    data_sq8: &[u8],
    sq8_mins: &[f32],
    sq8_scales: &[f32],
    dimension: usize,
    metric: &DistanceMetric,
) -> Vec<(u64, f32)> {
    let mut dequantized = vec![0.0f32; dimension];

    ids.iter()
        .enumerate()
        .map(|(idx, &id)| {
            let start = idx * dimension;
            let min = sq8_mins[idx];
            let scale = sq8_scales[idx];

            for (i, &q) in data_sq8[start..start + dimension].iter().enumerate() {
                dequantized[i] = (f32::from(q) / scale) + min;
            }

            let score = metric.calculate(query, &dequantized);
            (id, score)
        })
        .collect()
}

/// Compute scores for Binary quantized storage.
fn compute_scores_binary(
    query: &[f32],
    ids: &[u64],
    data_binary: &[u8],
    dimension: usize,
    metric: &DistanceMetric,
) -> Vec<(u64, f32)> {
    let bytes_per_vec = dimension.div_ceil(8);
    let mut binary_vec = vec![0.0f32; dimension];

    ids.iter()
        .enumerate()
        .map(|(idx, &id)| {
            let start = idx * bytes_per_vec;

            for (i, &byte) in data_binary[start..start + bytes_per_vec].iter().enumerate() {
                for bit in 0..8 {
                    let dim_idx = i * 8 + bit;
                    if dim_idx < dimension {
                        binary_vec[dim_idx] = if byte & (1 << bit) != 0 { 1.0 } else { 0.0 };
                    }
                }
            }

            let score = metric.calculate(query, &binary_vec);
            (id, score)
        })
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

/// Parse comparison operator string.
///
/// Supports: >, >=, <, <=, =, != (and aliases gt, gte, lt, lte, eq, neq)
#[allow(dead_code)]
pub fn parse_operator(op: &str) -> Option<Box<dyn Fn(f32, f32) -> bool>> {
    match op {
        ">" | "gt" => Some(Box::new(|score, thresh| score > thresh)),
        ">=" | "gte" => Some(Box::new(|score, thresh| score >= thresh)),
        "<" | "lt" => Some(Box::new(|score, thresh| score < thresh)),
        "<=" | "lte" => Some(Box::new(|score, thresh| score <= thresh)),
        "=" | "eq" => Some(Box::new(|score, thresh| (score - thresh).abs() < 0.001)),
        "!=" | "neq" => Some(Box::new(|score, thresh| (score - thresh).abs() >= 0.001)),
        _ => None,
    }
}

/// Compute scores with payload filtering.
#[allow(clippy::too_many_arguments)]
pub fn compute_filtered_scores<'a, F>(
    query: &[f32],
    ids: &[u64],
    payloads: &'a [Option<serde_json::Value>],
    data: &[f32],
    data_sq8: &[u8],
    data_binary: &[u8],
    sq8_mins: &[f32],
    sq8_scales: &[f32],
    dimension: usize,
    metric: &DistanceMetric,
    storage_mode: StorageMode,
    predicate: F,
) -> Vec<(u64, f32, Option<&'a serde_json::Value>)>
where
    F: Fn(&serde_json::Value) -> bool,
{
    match storage_mode {
        StorageMode::Full => ids
            .iter()
            .enumerate()
            .filter_map(|(idx, &id)| {
                let payload = payloads[idx].as_ref()?;
                if !predicate(payload) {
                    return None;
                }
                let start = idx * dimension;
                let v_data = &data[start..start + dimension];
                let score = metric.calculate(query, v_data);
                Some((id, score, Some(payload)))
            })
            .collect(),
        StorageMode::SQ8 => {
            let mut dequantized = vec![0.0f32; dimension];
            ids.iter()
                .enumerate()
                .filter_map(|(idx, &id)| {
                    let payload = payloads[idx].as_ref()?;
                    if !predicate(payload) {
                        return None;
                    }
                    let start = idx * dimension;
                    let min = sq8_mins[idx];
                    let scale = sq8_scales[idx];
                    for (i, &q) in data_sq8[start..start + dimension].iter().enumerate() {
                        dequantized[i] = (f32::from(q) / scale) + min;
                    }
                    let score = metric.calculate(query, &dequantized);
                    Some((id, score, Some(payload)))
                })
                .collect()
        }
        StorageMode::Binary => {
            let bytes_per_vec = dimension.div_ceil(8);
            let mut binary_vec = vec![0.0f32; dimension];
            ids.iter()
                .enumerate()
                .filter_map(|(idx, &id)| {
                    let payload = payloads[idx].as_ref()?;
                    if !predicate(payload) {
                        return None;
                    }
                    let start = idx * bytes_per_vec;
                    for (i, &byte) in data_binary[start..start + bytes_per_vec].iter().enumerate() {
                        for bit in 0..8 {
                            let dim_idx = i * 8 + bit;
                            if dim_idx < dimension {
                                binary_vec[dim_idx] =
                                    if byte & (1 << bit) != 0 { 1.0 } else { 0.0 };
                            }
                        }
                    }
                    let score = metric.calculate(query, &binary_vec);
                    Some((id, score, Some(payload)))
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_scores_full() {
        let query = [1.0, 0.0, 0.0, 0.0];
        let ids = vec![1, 2];
        let data = vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let metric = DistanceMetric::Cosine;

        let scores = compute_scores_full(&query, &ids, &data, 4, &metric);

        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0].0, 1);
        assert!((scores[0].1 - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_sort_results_higher() {
        let mut results = vec![(1, 0.5), (2, 0.9), (3, 0.3)];
        sort_results(&mut results, true);
        assert_eq!(results[0].0, 2);
    }

    #[test]
    fn test_sort_results_lower() {
        let mut results = vec![(1, 0.5), (2, 0.9), (3, 0.3)];
        sort_results(&mut results, false);
        assert_eq!(results[0].0, 3);
    }

    #[test]
    fn test_parse_operator() {
        assert!(parse_operator(">").is_some());
        assert!(parse_operator(">=").is_some());
        assert!(parse_operator("gt").is_some());
        assert!(parse_operator("invalid").is_none());
    }
}
