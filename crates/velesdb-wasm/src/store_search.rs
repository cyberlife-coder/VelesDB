//! Search operations for `VectorStore`.
//!
//! Provides search functionality extracted from the main store.

use crate::js_value::{to_js_value, PointId};
use crate::wasm_error::WasmError;
use crate::{fusion, text_search, vector_ops, DistanceMetric};
use serde::Serialize;
use velesdb_core::api_types::serde_id::id_to_json;
use wasm_bindgen::JsValue;

/// Serializes a search result through [`to_js_value`], rendering a failure
/// as a JS string error.
#[inline]
pub(crate) fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    to_js_value(value).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Dense hits as JavaScript receives them: `[[id, score], …]`, each id its
/// exact decimal string ([`PointId`]), each score a number.
///
/// The one shape of `search`, `search_with_quality`, `similarity_search`,
/// `multi_query_search` and `batch_search`.
pub(crate) fn scored_points(hits: Vec<(u64, f32)>) -> Vec<(PointId, f32)> {
    hits.into_iter()
        .map(|(id, score)| (PointId(id), score))
        .collect()
}

/// Scored triples `(id, score, payload)` as `{id, score, payload}` records,
/// the id its exact decimal string, as REST's `SearchResultResponse`.
///
/// Shared by `hybrid_search_impl` and `search_with_filter_impl`.
pub(crate) fn scored_triples_json(
    results: Vec<(u64, f32, Option<&serde_json::Value>)>,
) -> Vec<serde_json::Value> {
    results
        .into_iter()
        .map(|(id, score, payload)| {
            serde_json::json!({
                "id": id_to_json(id),
                "score": score,
                "payload": payload
            })
        })
        .collect()
}

/// [`scored_triples_json`], rendered for JavaScript.
pub(crate) fn scored_triples_to_js(
    results: Vec<(u64, f32, Option<&serde_json::Value>)>,
) -> Result<JsValue, JsValue> {
    to_js(&scored_triples_json(results))
}

/// Sorts `(id, score, payload)` triples by score according to metric ordering.
///
/// Reuses the same comparison logic as [`vector_ops::sort_results`] but operates
/// on triples that carry an optional payload reference.
pub(crate) fn sort_scored_triples<T>(results: &mut [(u64, f32, T)], higher_is_better: bool) {
    results.sort_by(|a, b| velesdb_core::distance::rank_scores(a.1, b.1, higher_is_better));
}

/// Validates query dimension matches store dimension, returning a structured
/// [`WasmError`] (carrying `VELES-004`) on mismatch.
///
/// Single source of truth for the dimension check: delegates to core's
/// [`velesdb_core::validate_dimension_match`] and preserves the core error's
/// machine-readable `code` instead of pre-flattening it to a bare string
/// (backlog #22). [`validate_query_vector`] and [`validate_ingest_vector`]
/// build on it; their callers render the error as a structured JS `Error`.
#[inline]
pub fn validate_dimension_structured(query_len: usize, store_dim: usize) -> Result<(), WasmError> {
    velesdb_core::validate_dimension_match(store_dim, query_len).map_err(WasmError::from)
}

/// The checks core runs on a query vector before it searches a collection:
/// the store holds vectors (a metadata-only store refuses with
/// `SearchNotSupported`, VELES-015), the query has the store's dimension
/// (VELES-004) and only finite components (`InvalidVector`, VELES-005).
pub fn validate_query_vector(query: &[f32], store_dim: usize) -> Result<(), WasmError> {
    refuse_metadata_only(store_dim)?;
    validate_ingest_vector(query, store_dim)
}

/// [`validate_query_vector`] for `num_vectors` queries laid out row-major in
/// one flat buffer.
pub fn validate_query_vectors(
    vectors: &[f32],
    num_vectors: usize,
    store_dim: usize,
) -> Result<(), WasmError> {
    refuse_metadata_only(store_dim)?;
    validate_multi_vector_len(vectors.len(), num_vectors, store_dim)
        .map_err(|e| WasmError::from(velesdb_core::Error::InvalidVector(e)))?;
    velesdb_core::validation::validate_vector_is_finite(vectors).map_err(WasmError::from)
}

/// The checks core runs on a vector it stores: the store's dimension and
/// only finite components.
pub fn validate_ingest_vector(vector: &[f32], store_dim: usize) -> Result<(), WasmError> {
    validate_dimension_structured(vector.len(), store_dim)?;
    velesdb_core::validation::validate_vector_is_finite(vector).map_err(WasmError::from)
}

/// A store of dimension 0 is metadata-only: it holds no vector to search.
fn refuse_metadata_only(store_dim: usize) -> Result<(), WasmError> {
    if store_dim == 0 {
        return Err(WasmError::from(velesdb_core::Error::SearchNotSupported(
            "vector search needs a vector store; this one is metadata-only".to_string(),
        )));
    }
    Ok(())
}

/// Validates that a flat multi-vector buffer holds exactly
/// `num_vectors * dimension` floats, using a checked multiplication.
///
/// On `wasm32`, `usize` is 32-bit, so an unchecked `num_vectors * dimension`
/// can wrap and spoof the length comparison — admitting an out-of-bounds slice
/// when the per-vector chunks are later read. `checked_mul` rejects the
/// overflow up front. Returns the validated expected length on success.
pub fn validate_multi_vector_len(
    actual_len: usize,
    num_vectors: usize,
    dimension: usize,
) -> Result<usize, String> {
    let expected = num_vectors.checked_mul(dimension).ok_or_else(|| {
        "Vector array size overflow: num_vectors * dimension exceeds usize".to_string()
    })?;
    if actual_len != expected {
        return Err(format!(
            "Vector array size mismatch: expected {expected}, got {actual_len}"
        ));
    }
    Ok(expected)
}

/// Top-`k` `(id, score)` pairs for `query`, best first.
pub fn top_k(
    query: &[f32],
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: DistanceMetric,
    k: usize,
) -> Vec<(u64, f32)> {
    let mut results = vector_ops::compute_scores(query, ids, data, dimension, metric);
    vector_ops::sort_results(&mut results, metric.higher_is_better());
    results.truncate(k);
    results
}

/// Text search in payloads.
///
/// A case-insensitive substring match: it selects records, it does not rank
/// them. Matches come back in insertion order with `score: null` — the
/// `{id, score, payload}` shape of the other record-returning searches, with
/// no number a caller could mistake for relevance and threshold or sort on.
pub fn text_search_impl(
    query: &str,
    ids: &[u64],
    payloads: &[Option<serde_json::Value>],
    field: Option<&str>,
    k: usize,
) -> Result<JsValue, JsValue> {
    to_js(&text_search_matches(query, ids, payloads, field, k))
}

/// The records behind [`text_search_impl`], each id its exact decimal string.
pub(crate) fn text_search_matches(
    query: &str,
    ids: &[u64],
    payloads: &[Option<serde_json::Value>],
    field: Option<&str>,
    k: usize,
) -> Vec<serde_json::Value> {
    let query_lower = query.to_lowercase();

    ids.iter()
        .zip(payloads.iter())
        .filter_map(|(&id, payload)| {
            payload.as_ref().and_then(|p| {
                let matches = match field {
                    Some(f) => text_search::payload_contains_text(p, &query_lower, Some(f)),
                    None => text_search::search_all_fields(p, &query_lower),
                };

                if matches {
                    Some(serde_json::json!({
                        "id": id_to_json(id),
                        "score": serde_json::Value::Null,
                        "payload": p
                    }))
                } else {
                    None
                }
            })
        })
        .take(k)
        .collect()
}

/// Scored `(id, combined_score, payload)` triples behind
/// [`hybrid_search_impl`] — native-typed so host tests can pin the exact
/// browser-store contract without a `JsValue` (which needs the wasm32
/// runtime).
#[allow(clippy::too_many_arguments)]
pub fn hybrid_search_scored<'a>(
    query_vector: &[f32],
    text_query: &str,
    ids: &[u64],
    data: &[f32],
    payloads: &'a [Option<serde_json::Value>],
    dimension: usize,
    metric: DistanceMetric,
    k: usize,
    vector_weight: Option<f32>,
) -> Vec<(u64, f32, Option<&'a serde_json::Value>)> {
    let v_weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
    let t_weight = 1.0 - v_weight;
    let text_query_lower = text_query.to_lowercase();
    let higher_is_better = metric.higher_is_better();

    let mut results: Vec<(u64, f32, Option<&serde_json::Value>)> = ids
        .iter()
        .enumerate()
        .filter_map(|(idx, &id)| {
            let start = idx * dimension;
            let v_data = &data[start..start + dimension];
            // A distance is negated so the blend is higher-is-better under
            // every metric, as core's fusion orients a lower-is-better branch.
            let raw = metric.calculate(query_vector, v_data);
            let vector_score = if higher_is_better { raw } else { -raw };

            let payload = payloads[idx].as_ref();
            let text_score = payload.map_or(0.0, |p| {
                if text_search::search_all_fields(p, &text_query_lower) {
                    1.0
                } else {
                    0.0
                }
            });

            let combined_score = v_weight * vector_score + t_weight * text_score;
            // A row needs a weighted branch speaking for it: with the vector
            // weighted 0 the search is text-only, under every metric. Above
            // that, the relevance floor is a similarity's: a negated distance
            // is never above it, so a distance metric keeps every row.
            let ranked = v_weight > 0.0 || text_score > 0.0;
            if ranked && (!higher_is_better || combined_score > 0.0) {
                Some((id, combined_score, payload))
            } else {
                None
            }
        })
        .collect();

    sort_scored_triples(&mut results, true);
    results.truncate(k);
    results
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
    scored_triples_to_js(hybrid_search_scored(
        query_vector,
        text_query,
        ids,
        data,
        payloads,
        dimension,
        metric,
        k,
        vector_weight,
    ))
}

/// Converts an optional flat `[avg_weight, max_weight, hit_weight]` slice
/// from the JS boundary into the `(f32, f32, f32)` tuple `fuse_results`
/// expects for the `"weighted"` strategy.
///
/// `None` means "use core's canonical default weights". A slice whose length
/// isn't exactly 3 is a caller error, not a silent fallback.
fn weighted_weights_from_slice(
    weights: Option<&[f32]>,
) -> Result<Option<(f32, f32, f32)>, JsValue> {
    match weights {
        None => Ok(None),
        Some([avg, max, hit]) => Ok(Some((*avg, *max, *hit))),
        Some(other) => Err(JsValue::from_str(&format!(
            "weighted fusion expects exactly 3 weights [avg_weight, max_weight, hit_weight], got {}",
            other.len()
        ))),
    }
}

/// Multi-query search with fusion.
#[allow(clippy::too_many_arguments)]
pub fn multi_query_search_impl(
    vectors: &[f32],
    num_vectors: usize,
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: DistanceMetric,
    k: usize,
    strategy: &str,
    rrf_k: Option<u32>,
    weights: Option<&[f32]>,
) -> Result<JsValue, JsValue> {
    validate_multi_vector_len(vectors.len(), num_vectors, dimension)
        .map_err(|e| JsValue::from_str(&e))?;
    // `weights` is strategy-conditional: `"weighted"` reads its
    // [avg, max, hit] triple, `"weighted_rrf"` and `"relative_score"` /
    // `"rsf"` read one weight per query vector, and every other strategy
    // ignores it.
    let per_branch = ["weighted_rrf", "relative_score", "rsf"]
        .iter()
        .any(|name| strategy.eq_ignore_ascii_case(name));
    let (weights, branch_weights) = if per_branch {
        let branch_weights = match weights {
            Some(w) if w.len() == num_vectors => Some(w),
            Some(w) => {
                return Err(JsValue::from_str(&format!(
                "{strategy} fusion expects one weight per query vector: {} weights for {} vectors",
                w.len(),
                num_vectors
            )))
            }
            // `None` reaches core as missing weights — the named refusal,
            // not an invented default.
            None => None,
        };
        (None, branch_weights)
    } else {
        (weighted_weights_from_slice(weights)?, None)
    };

    // `k * 2` wraps silently in release / panics in debug, and the retained
    // `num_vectors × k` fan-out is caller-controlled — both bounded up front
    // by the browser guardrails (#2679).
    let fanout_k =
        crate::guardrails::checked_fanout_k(k).map_err(|e| WasmError::from(e).into_js_value())?;
    crate::guardrails::check_fanout_memory(num_vectors, fanout_k.min(ids.len()))
        .map_err(|e| WasmError::from(e).into_js_value())?;

    let mut all_results: Vec<Vec<(u64, f32)>> = Vec::with_capacity(num_vectors);

    for query in vectors.chunks_exact(dimension) {
        all_results.push(top_k(query, ids, data, dimension, metric, fanout_k));
    }

    let fused = fusion::fuse_results(
        &all_results,
        strategy,
        rrf_k.unwrap_or(60),
        weights,
        branch_weights,
        metric,
    )
    .map_err(|e| JsValue::from_str(&e))?;
    let fused: Vec<(u64, f32)> = fused.into_iter().take(k).collect();

    to_js(&scored_points(fused))
}

/// Search with metadata filter.
#[allow(clippy::too_many_arguments)]
pub fn search_with_filter_impl<F>(
    query: &[f32],
    ids: &[u64],
    payloads: &[Option<serde_json::Value>],
    data: &[f32],
    dimension: usize,
    metric: crate::DistanceMetric,
    k: usize,
    filter_fn: F,
) -> Result<JsValue, JsValue>
where
    F: Fn(u64, Option<&serde_json::Value>) -> bool,
{
    let mut results = crate::vector_ops::compute_filtered_scores(
        query, ids, payloads, data, dimension, metric, filter_fn,
    );

    sort_scored_triples(&mut results, metric.higher_is_better());
    results.truncate(k);

    scored_triples_to_js(results)
}

/// Similarity search with threshold filtering.
#[allow(clippy::too_many_arguments)]
pub fn similarity_search_impl(
    query: &[f32],
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: crate::DistanceMetric,
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

    let all_scores = vector_ops::compute_scores(query, ids, data, dimension, metric);

    let mut results: Vec<(u64, f32)> = all_scores
        .into_iter()
        .filter(|(_, score)| op_fn(*score, threshold))
        .collect();

    vector_ops::sort_results(&mut results, higher_is_better);
    results.truncate(k);

    to_js(&scored_points(results))
}

/// Batch search for multiple vectors.
pub fn batch_search_impl(
    vectors: &[f32],
    num_vectors: usize,
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: crate::DistanceMetric,
    k: usize,
) -> Result<JsValue, JsValue> {
    // Retained fan-out `num_vectors × k` is caller-controlled — bounded up
    // front by the browser guardrails (#2679).
    crate::guardrails::check_fanout_memory(num_vectors, k.min(ids.len()))
        .map_err(|e| WasmError::from(e).into_js_value())?;
    let all_results: Vec<Vec<(PointId, f32)>> = vectors
        .chunks_exact(dimension)
        .map(|query| scored_points(top_k(query, ids, data, dimension, metric, k)))
        .collect();
    to_js(&all_results)
}

#[cfg(test)]
#[path = "store_guards_tests.rs"]
mod guards_tests;
