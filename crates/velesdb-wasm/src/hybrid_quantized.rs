//! Hybrid search for quantized storage modes (SQ8/Binary/PQ).
//!
//! Extracted from `vector_store.rs` to keep module size under 500 NLOC.

use wasm_bindgen::prelude::*;

use crate::store_search;
use crate::text_search;
use crate::vector_ops;
use crate::{DistanceMetric, StorageMode};

/// Decomposed hybrid search for quantized storage modes.
///
/// Computes vector scores via `compute_scores` (which handles SQ8/Binary/PQ
/// dequantization internally), evaluates text matches on payloads, and fuses
/// the two signal sources with the requested weight.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hybrid_search_quantized(
    query_vector: &[f32],
    text_query: &str,
    k: usize,
    vector_weight: Option<f32>,
    ids: &[u64],
    data: &[f32],
    data_sq8: &[u8],
    data_binary: &[u8],
    sq8_mins: &[f32],
    sq8_scales: &[f32],
    payloads: &[Option<serde_json::Value>],
    dimension: usize,
    metric: DistanceMetric,
    storage_mode: StorageMode,
) -> Result<JsValue, JsValue> {
    let v_weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
    let t_weight = 1.0 - v_weight;
    let text_query_lower = text_query.to_lowercase();

    // Vector scores via the quantization-aware path (covers all storage modes).
    let vector_scores = vector_ops::compute_scores(
        query_vector,
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

    // Build text-match lookup from payloads (independent of quantization).
    let text_matches: std::collections::HashSet<u64> = ids
        .iter()
        .zip(payloads.iter())
        .filter_map(|(&id, payload)| {
            payload.as_ref().and_then(|p| {
                if text_search::search_all_fields(p, &text_query_lower) {
                    Some(id)
                } else {
                    None
                }
            })
        })
        .collect();

    // Build id-to-index map for payload lookup.
    let id_to_idx: std::collections::HashMap<u64, usize> =
        ids.iter().enumerate().map(|(idx, &id)| (id, idx)).collect();

    // Fuse: every ID in vector_scores already has a vector score; add text
    // contribution when the ID also appears in text_matches.
    let mut results: Vec<(u64, f32, Option<&serde_json::Value>)> = vector_scores
        .into_iter()
        .filter_map(|(id, vscore)| {
            let text_score = if text_matches.contains(&id) {
                1.0_f32
            } else {
                0.0
            };
            let combined = v_weight * vscore + t_weight * text_score;
            if combined > 0.0 {
                let payload = id_to_idx
                    .get(&id)
                    .and_then(|&idx| payloads.get(idx).and_then(Option::as_ref));
                Some((id, combined, payload))
            } else {
                None
            }
        })
        .collect();

    // Sort descending by combined score (hybrid always uses higher-is-better).
    store_search::sort_scored_triples(&mut results, true);
    results.truncate(k);

    store_search::scored_triples_to_js(results)
}
