//! Search operations for VectorStore.
//!
//! This module contains search helpers extracted from lib.rs to reduce file size.

use crate::distance::DistanceMetric;
use crate::filter;
use crate::fusion;
use crate::text_search;
use crate::vector_ops;
use crate::StorageMode;

/// Reference to VectorStore data for search operations.
pub struct StoreRef<'a> {
    pub ids: &'a [u64],
    pub data: &'a [f32],
    pub data_sq8: &'a [u8],
    pub data_binary: &'a [u8],
    pub sq8_mins: &'a [f32],
    pub sq8_scales: &'a [f32],
    pub payloads: &'a [Option<serde_json::Value>],
    pub dimension: usize,
    pub metric: &'a DistanceMetric,
    pub storage_mode: StorageMode,
}

/// Performs k-NN search and returns (id, score) pairs.
pub fn search_knn(store: &StoreRef<'_>, query: &[f32], k: usize) -> Vec<(u64, f32)> {
    let mut results = vector_ops::compute_scores(
        query,
        store.ids,
        store.data,
        store.data_sq8,
        store.data_binary,
        store.sq8_mins,
        store.sq8_scales,
        store.dimension,
        store.metric,
        store.storage_mode,
    );

    vector_ops::sort_results(&mut results, store.metric.higher_is_better());
    results.truncate(k);
    results
}

/// Performs filtered search with metadata predicate.
pub fn search_with_filter_impl<'a>(
    store: &StoreRef<'a>,
    query: &[f32],
    k: usize,
    filter_obj: &serde_json::Value,
) -> Vec<(u64, f32, Option<&'a serde_json::Value>)> {
    let mut results = vector_ops::compute_filtered_scores(
        query,
        store.ids,
        store.payloads,
        store.data,
        store.data_sq8,
        store.data_binary,
        store.sq8_mins,
        store.sq8_scales,
        store.dimension,
        store.metric,
        store.storage_mode,
        |payload| filter::matches_filter(payload, filter_obj),
    );

    // Sort by relevance
    results.sort_by(|a, b| {
        if store.metric.higher_is_better() {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
        }
    });
    results.truncate(k);
    results
}

/// Similarity search with threshold filtering.
pub fn similarity_search_impl(
    store: &StoreRef<'_>,
    query: &[f32],
    threshold: f32,
    op_fn: &dyn Fn(f32, f32) -> bool,
    k: usize,
) -> Vec<(u64, f32)> {
    let all_scores = vector_ops::compute_scores(
        query,
        store.ids,
        store.data,
        store.data_sq8,
        store.data_binary,
        store.sq8_mins,
        store.sq8_scales,
        store.dimension,
        store.metric,
        store.storage_mode,
    );

    let mut results: Vec<(u64, f32)> = all_scores
        .into_iter()
        .filter(|(_, score)| op_fn(*score, threshold))
        .collect();

    vector_ops::sort_results(&mut results, store.metric.higher_is_better());
    results.truncate(k);
    results
}

/// Text search on payload fields.
pub fn text_search_impl<'a>(
    ids: &'a [u64],
    payloads: &'a [Option<serde_json::Value>],
    query: &str,
    k: usize,
    field: Option<&str>,
) -> Vec<(u64, f32, Option<&'a serde_json::Value>)> {
    let query_lower = query.to_lowercase();

    let mut results: Vec<(u64, f32, Option<&serde_json::Value>)> = ids
        .iter()
        .enumerate()
        .filter_map(|(idx, &id)| {
            let payload = payloads[idx].as_ref()?;
            let matches = text_search::payload_contains_text(payload, &query_lower, field);
            if matches {
                Some((id, 1.0, Some(payload)))
            } else {
                None
            }
        })
        .collect();

    results.truncate(k);
    results
}

/// Hybrid search combining vector and text.
pub fn hybrid_search_impl<'a>(
    store: &StoreRef<'a>,
    query_vector: &[f32],
    text_query: &str,
    k: usize,
    vector_weight: f32,
) -> Vec<(u64, f32, Option<&'a serde_json::Value>)> {
    let t_weight = 1.0 - vector_weight;
    let text_query_lower = text_query.to_lowercase();

    // Only Full mode for hybrid search
    if store.storage_mode != StorageMode::Full {
        return Vec::new();
    }

    let mut results: Vec<(u64, f32, Option<&serde_json::Value>)> = store
        .ids
        .iter()
        .enumerate()
        .filter_map(|(idx, &id)| {
            let start = idx * store.dimension;
            let v_data = &store.data[start..start + store.dimension];
            let vector_score = store.metric.calculate(query_vector, v_data);

            let payload = store.payloads[idx].as_ref();
            let text_score = if let Some(p) = payload {
                if text_search::search_all_fields(p, &text_query_lower) {
                    1.0
                } else {
                    0.0
                }
            } else {
                0.0
            };

            let combined_score = vector_weight * vector_score + t_weight * text_score;
            if combined_score > 0.0 {
                Some((id, combined_score, payload))
            } else {
                None
            }
        })
        .collect();

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(k);
    results
}

/// Multi-query search with result fusion.
pub fn multi_query_search_impl(
    store: &StoreRef<'_>,
    vectors: &[f32],
    num_vectors: usize,
    k: usize,
    strategy: &str,
    rrf_k: u32,
) -> Vec<(u64, f32)> {
    // Only Full mode supported
    if store.storage_mode != StorageMode::Full {
        return Vec::new();
    }

    let overfetch_k = k * 3;
    let mut all_results: Vec<Vec<(u64, f32)>> = Vec::with_capacity(num_vectors);

    for i in 0..num_vectors {
        let start = i * store.dimension;
        let query = &vectors[start..start + store.dimension];

        let mut r: Vec<(u64, f32)> = store
            .ids
            .iter()
            .enumerate()
            .map(|(idx, &id)| {
                let v_start = idx * store.dimension;
                let v_data = &store.data[v_start..v_start + store.dimension];
                let score = store.metric.calculate(query, v_data);
                (id, score)
            })
            .collect();

        if store.metric.higher_is_better() {
            r.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            r.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        r.truncate(overfetch_k);
        all_results.push(r);
    }

    let fused = fusion::fuse_results(&all_results, strategy, rrf_k);
    fused.into_iter().take(k).collect()
}

/// Batch search for multiple vectors.
pub fn batch_search_impl(
    store: &StoreRef<'_>,
    vectors: &[f32],
    num_vectors: usize,
    k: usize,
) -> Vec<Vec<(u64, f32)>> {
    // Only Full mode supported for now
    if store.storage_mode != StorageMode::Full {
        return vec![Vec::new(); num_vectors];
    }

    let mut all_results: Vec<Vec<(u64, f32)>> = Vec::with_capacity(num_vectors);

    for i in 0..num_vectors {
        let start = i * store.dimension;
        let query = &vectors[start..start + store.dimension];

        let mut r: Vec<(u64, f32)> = store
            .ids
            .iter()
            .enumerate()
            .map(|(idx, &id)| {
                let v_start = idx * store.dimension;
                let v_data = &store.data[v_start..v_start + store.dimension];
                (id, store.metric.calculate(query, v_data))
            })
            .collect();

        if store.metric.higher_is_better() {
            r.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            r.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        r.truncate(k);
        all_results.push(r);
    }

    all_results
}

/// Parse comparison operator for similarity search.
pub fn parse_similarity_operator(op: &str) -> Option<Box<dyn Fn(f32, f32) -> bool>> {
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
