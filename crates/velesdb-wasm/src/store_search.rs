//! Search operations for `VectorStore`.
//!
//! Provides search functionality extracted from the main store.

use crate::{text_search, vector_ops, DistanceMetric, StorageMode};
use velesdb_core::fusion::FusionStrategy;
use wasm_bindgen::JsValue;

/// Validates query dimension matches store dimension.
#[inline]
pub fn validate_dimension(query_len: usize, store_dim: usize) -> Result<(), JsValue> {
    if query_len != store_dim {
        return Err(JsValue::from_str(&format!(
            "Query dimension mismatch: expected {store_dim}, got {query_len}"
        )));
    }
    Ok(())
}

/// Basic vector search.
#[allow(clippy::too_many_arguments)]
pub fn search(
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
    k: usize,
) -> Result<JsValue, JsValue> {
    let mut results = vector_ops::compute_scores(
        query,
        ids,
        data,
        data_sq8,
        data_binary,
        sq8_mins,
        sq8_scales,
        dimension,
        metric,
        storage_mode,
    );

    vector_ops::sort_results(&mut results, metric.higher_is_better());
    results.truncate(k);

    serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Text search in payloads.
pub fn text_search_impl(
    query: &str,
    ids: &[u64],
    payloads: &[Option<serde_json::Value>],
    field: Option<&str>,
    k: usize,
) -> Result<JsValue, JsValue> {
    let query_lower = query.to_lowercase();

    let results: Vec<serde_json::Value> = ids
        .iter()
        .zip(payloads.iter())
        .filter_map(|(&id, payload)| {
            payload.as_ref().and_then(|p| {
                let matches = match field {
                    Some(f) => text_search::payload_contains_text(p, &query_lower, Some(f)),
                    None => text_search::search_all_fields(p, &query_lower),
                };

                if matches {
                    Some(serde_json::json!({
                        "id": id,
                        "payload": p
                    }))
                } else {
                    None
                }
            })
        })
        .take(k)
        .collect();

    serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Hybrid vector + text search.
#[allow(clippy::too_many_arguments)]
pub fn hybrid_search_impl(
    query_vector: &[f32],
    text_query: &str,
    ids: &[u64],
    data: &[f32],
    payloads: &[Option<serde_json::Value>],
    dimension: usize,
    metric: DistanceMetric,
    k: usize,
    vector_weight: Option<f32>,
) -> Result<JsValue, JsValue> {
    let v_weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
    let t_weight = 1.0 - v_weight;
    let text_query_lower = text_query.to_lowercase();

    let mut results: Vec<(u64, f32, Option<&serde_json::Value>)> = ids
        .iter()
        .enumerate()
        .filter_map(|(idx, &id)| {
            let start = idx * dimension;
            let v_data = &data[start..start + dimension];
            let vector_score = metric.calculate(query_vector, v_data);

            let payload = payloads[idx].as_ref();
            let text_score = payload.map_or(0.0, |p| {
                if text_search::search_all_fields(p, &text_query_lower) {
                    1.0
                } else {
                    0.0
                }
            });

            let combined_score = v_weight * vector_score + t_weight * text_score;
            if combined_score > 0.0 {
                Some((id, combined_score, payload))
            } else {
                None
            }
        })
        .collect();

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(k);

    let output: Vec<serde_json::Value> = results
        .into_iter()
        .map(|(id, score, payload)| {
            serde_json::json!({
                "id": id,
                "score": score,
                "payload": payload
            })
        })
        .collect();

    serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Multi-query search with fusion.
#[allow(clippy::too_many_arguments)]
pub fn multi_query_search_impl(
    vectors: &[f32],
    num_vectors: usize,
    ids: &[u64],
    data: &[f32],
    data_sq8: &[u8],
    data_binary: &[u8],
    sq8_mins: &[f32],
    sq8_scales: &[f32],
    dimension: usize,
    metric: DistanceMetric,
    storage_mode: StorageMode,
    k: usize,
    strategy: &str,
    rrf_k: Option<u32>,
) -> Result<JsValue, JsValue> {
    if vectors.len() != num_vectors * dimension {
        return Err(JsValue::from_str(&format!(
            "Vector array size mismatch: expected {}, got {}",
            num_vectors * dimension,
            vectors.len()
        )));
    }

    let mut all_results: Vec<Vec<(u64, f32)>> = Vec::with_capacity(num_vectors);

    for i in 0..num_vectors {
        let start = i * dimension;
        let query = &vectors[start..start + dimension];

        let mut results = vector_ops::compute_scores(
            query,
            ids,
            data,
            data_sq8,
            data_binary,
            sq8_mins,
            sq8_scales,
            dimension,
            metric,
            storage_mode,
        );

        vector_ops::sort_results(&mut results, metric.higher_is_better());
        results.truncate(k * 2);
        all_results.push(results);
    }

    let fusion_strategy = match strategy {
        "average" | "avg" => FusionStrategy::Average,
        "maximum" | "max" => FusionStrategy::Maximum,
        _ => FusionStrategy::RRF {
            k: rrf_k.unwrap_or(60),
        },
    };
    let fused = fusion_strategy.fuse(all_results).unwrap_or_default();
    let fused: Vec<(u64, f32)> = fused.into_iter().take(k).collect();

    serde_wasm_bindgen::to_value(&fused).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Search with metadata filter.
#[allow(clippy::too_many_arguments)]
pub fn search_with_filter_impl<F>(
    query: &[f32],
    ids: &[u64],
    payloads: &[Option<serde_json::Value>],
    data: &[f32],
    data_sq8: &[u8],
    data_binary: &[u8],
    sq8_mins: &[f32],
    sq8_scales: &[f32],
    dimension: usize,
    metric: crate::DistanceMetric,
    storage_mode: crate::StorageMode,
    k: usize,
    filter_fn: F,
) -> Result<JsValue, JsValue>
where
    F: Fn(&serde_json::Value) -> bool,
{
    let mut results = crate::vector_ops::compute_filtered_scores(
        query,
        ids,
        payloads,
        data,
        data_sq8,
        data_binary,
        sq8_mins,
        sq8_scales,
        dimension,
        metric,
        storage_mode,
        filter_fn,
    );

    results.sort_by(|a, b| {
        if metric.higher_is_better() {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
        }
    });
    results.truncate(k);

    let output: Vec<serde_json::Value> = results
        .into_iter()
        .map(|(id, score, payload)| {
            serde_json::json!({
                "id": id,
                "score": score,
                "payload": payload
            })
        })
        .collect();

    serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Similarity search with threshold filtering.
#[allow(clippy::too_many_arguments)]
pub fn similarity_search_impl(
    query: &[f32],
    ids: &[u64],
    data: &[f32],
    data_sq8: &[u8],
    data_binary: &[u8],
    sq8_mins: &[f32],
    sq8_scales: &[f32],
    dimension: usize,
    metric: crate::DistanceMetric,
    storage_mode: crate::StorageMode,
    threshold: f32,
    operator: &str,
    k: usize,
) -> Result<JsValue, JsValue> {
    let higher_is_better = metric.higher_is_better();
    let op_fn: Box<dyn Fn(f32, f32) -> bool> = match (operator, higher_is_better) {
        (">" | "gt", true) => Box::new(|score, thresh| score > thresh),
        (">=" | "gte", true) => Box::new(|score, thresh| score >= thresh),
        ("<" | "lt", true) => Box::new(|score, thresh| score < thresh),
        ("<=" | "lte", true) => Box::new(|score, thresh| score <= thresh),
        (">" | "gt", false) => Box::new(|score, thresh| score < thresh),
        (">=" | "gte", false) => Box::new(|score, thresh| score <= thresh),
        ("<" | "lt", false) => Box::new(|score, thresh| score > thresh),
        ("<=" | "lte", false) => Box::new(|score, thresh| score >= thresh),
        ("=" | "eq", _) => Box::new(|score, thresh| (score - thresh).abs() < 0.001),
        ("!=" | "neq", _) => Box::new(|score, thresh| (score - thresh).abs() >= 0.001),
        _ => {
            return Err(JsValue::from_str(
                "Invalid operator. Use: >, >=, <, <=, =, !=",
            ));
        }
    };

    let all_scores = vector_ops::compute_scores(
        query,
        ids,
        data,
        data_sq8,
        data_binary,
        sq8_mins,
        sq8_scales,
        dimension,
        metric,
        storage_mode,
    );

    let mut results: Vec<(u64, f32)> = all_scores
        .into_iter()
        .filter(|(_, score)| op_fn(*score, threshold))
        .collect();

    vector_ops::sort_results(&mut results, higher_is_better);
    results.truncate(k);

    serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Batch search for multiple vectors (Full storage mode only).
#[allow(clippy::too_many_arguments)]
pub fn batch_search_impl(
    vectors: &[f32],
    num_vectors: usize,
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: crate::DistanceMetric,
    k: usize,
) -> Result<JsValue, JsValue> {
    let mut all_results: Vec<Vec<(u64, f32)>> = Vec::with_capacity(num_vectors);

    for i in 0..num_vectors {
        let start = i * dimension;
        let query = &vectors[start..start + dimension];

        let mut results: Vec<(u64, f32)> = ids
            .iter()
            .enumerate()
            .map(|(idx, &id)| {
                let v_start = idx * dimension;
                let v_data = &data[v_start..v_start + dimension];
                (id, metric.calculate(query, v_data))
            })
            .collect();

        vector_ops::sort_results(&mut results, metric.higher_is_better());
        results.truncate(k);
        all_results.push(results);
    }

    serde_wasm_bindgen::to_value(&all_results).map_err(|e| JsValue::from_str(&e.to_string()))
}
