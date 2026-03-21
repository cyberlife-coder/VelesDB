//! Internal vector operations for `VectorStore`.
//!
//! This module contains extracted scoring and search logic to reduce lib.rs size.
//! These are internal helpers, not exposed via `wasm_bindgen`.

use crate::StorageMode;
use velesdb_core::DistanceMetric;

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
    metric: DistanceMetric,
    storage_mode: StorageMode,
) -> Vec<(u64, f32)> {
    let mut scratch = ScratchBuffer::new(dimension, storage_mode);

    ids.iter()
        .enumerate()
        .map(|(idx, &id)| {
            let v = scratch.dequantize(
                idx,
                data,
                data_sq8,
                data_binary,
                sq8_mins,
                sq8_scales,
                dimension,
                storage_mode,
            );
            (id, metric.calculate(query, v))
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
    metric: DistanceMetric,
    storage_mode: StorageMode,
    predicate: F,
) -> Vec<(u64, f32, Option<&'a serde_json::Value>)>
where
    F: Fn(&serde_json::Value) -> bool,
{
    let mut scratch = ScratchBuffer::new(dimension, storage_mode);

    ids.iter()
        .enumerate()
        .filter_map(|(idx, &id)| {
            let payload = payloads[idx].as_ref()?;
            if !predicate(payload) {
                return None;
            }
            let v = scratch.dequantize(
                idx,
                data,
                data_sq8,
                data_binary,
                sq8_mins,
                sq8_scales,
                dimension,
                storage_mode,
            );
            let score = metric.calculate(query, v);
            Some((id, score, Some(payload)))
        })
        .collect()
}

/// Reusable scratch buffer for dequantizing vectors across storage modes.
///
/// Avoids per-vector allocation for SQ8 and Binary modes by reusing a
/// pre-allocated buffer that is overwritten on each `dequantize` call.
struct ScratchBuffer {
    buf: Vec<f32>,
    bytes_per_vec: usize,
}

impl ScratchBuffer {
    /// Creates a scratch buffer sized for the given dimension and storage mode.
    #[must_use]
    fn new(dimension: usize, mode: StorageMode) -> Self {
        let needs_buf = matches!(
            mode,
            StorageMode::SQ8 | StorageMode::Binary | StorageMode::ProductQuantization
        );
        Self {
            buf: if needs_buf {
                vec![0.0f32; dimension]
            } else {
                Vec::new()
            },
            bytes_per_vec: dimension.div_ceil(8),
        }
    }

    /// Dequantizes vector at `idx` and returns a slice to the data.
    ///
    /// For `Full` mode, returns a direct slice into `data` (zero-copy).
    /// For `SQ8`/`Binary`/`PQ`, writes into the internal scratch buffer and
    /// returns a slice to it.
    #[allow(clippy::too_many_arguments)]
    fn dequantize<'a>(
        &'a mut self,
        idx: usize,
        data: &'a [f32],
        data_sq8: &[u8],
        data_binary: &[u8],
        sq8_mins: &[f32],
        sq8_scales: &[f32],
        dimension: usize,
        mode: StorageMode,
    ) -> &'a [f32] {
        match mode {
            StorageMode::Full => {
                let start = idx * dimension;
                &data[start..start + dimension]
            }
            // ProductQuantization shares the SQ8 decode path because WASM-side PQ
            // centroids are pre-decoded to per-vector SQ8 codes during ingestion.
            StorageMode::SQ8 | StorageMode::ProductQuantization => {
                let start = idx * dimension;
                let min = sq8_mins[idx];
                let scale = sq8_scales[idx];
                for (i, &q) in data_sq8[start..start + dimension].iter().enumerate() {
                    self.buf[i] = (f32::from(q) / scale) + min;
                }
                &self.buf[..dimension]
            }
            StorageMode::Binary => {
                let start = idx * self.bytes_per_vec;
                for (i, &byte) in data_binary[start..start + self.bytes_per_vec]
                    .iter()
                    .enumerate()
                {
                    for bit in 0..8 {
                        let dim_idx = i * 8 + bit;
                        if dim_idx < dimension {
                            self.buf[dim_idx] = if byte & (1 << bit) != 0 { 1.0 } else { 0.0 };
                        }
                    }
                }
                &self.buf[..dimension]
            }
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

        let scores = compute_scores(
            &query,
            &ids,
            &data,
            &[],
            &[],
            &[],
            &[],
            4,
            metric,
            StorageMode::Full,
        );

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
}
