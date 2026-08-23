//! Internal helpers for CRUD operations: PQ quantization caching, secondary
//! index updates, and `DedupMap`.

use crate::collection::types::Collection;
use crate::index::{JsonValue, SecondaryIndex};
use crate::point::Point;
use crate::quantization::{PQVector, ProductQuantizer, StorageMode};
use parking_lot::RwLockWriteGuard;
use std::collections::HashMap;

const PQ_TRAINING_SAMPLES: usize = 128;

/// Pre-computed last-writer-wins dedup map: `point_id -> index_of_last_occurrence`.
///
/// Built once in `batch_store_all` and shared by both `write_deduped_payloads`
/// and `write_deduped_vectors` to avoid redundant map construction (Issue #425).
pub(super) type DedupMap = HashMap<u64, usize>;

/// Write-lock guard over the PQ cache, acquired once per batch.
///
/// `ProductQuantization` is the only storage mode with per-upsert quantization
/// work: its quantizer carries shared lazy-training state, so points are
/// processed sequentially under one guard. (SQ8/Binary used to fill sibling
/// caches here, but nothing ever read them — see `StorageMode`'s docs.)
pub(super) type PqCacheGuard<'a> = RwLockWriteGuard<'a, HashMap<u64, PQVector>>;

/// Acquires the PQ cache guard when `mode` needs it, `None` otherwise.
pub(super) fn pq_cache_guard(
    collection: &Collection,
    mode: StorageMode,
) -> Option<PqCacheGuard<'_>> {
    matches!(mode, StorageMode::ProductQuantization).then(|| collection.storage.pq_cache.write())
}

fn auto_num_subspaces(dimension: usize) -> usize {
    let mut num_subspaces = 8usize;
    while num_subspaces > 1 && !dimension.is_multiple_of(num_subspaces) {
        num_subspaces /= 2;
    }
    num_subspaces.max(1)
}

impl Collection {
    /// Caches the PQ code for `point` (ProductQuantization mode only).
    ///
    /// Trains the quantizer on first `PQ_TRAINING_SAMPLES` points, then
    /// backfills and quantizes subsequent points.
    pub(super) fn cache_pq_vector(
        &self,
        point: &Point,
        pq_cache: Option<&mut std::collections::HashMap<u64, PQVector>>,
    ) {
        let mut quantizer_guard = self.storage.pq_quantizer.write();
        let mut backfill_samples: Vec<(u64, Vec<f32>)> = Vec::new();

        if quantizer_guard.is_none() {
            let mut buffer = self.storage.pq_training_buffer.write();
            buffer.push_back((point.id, point.vector.clone()));
            if buffer.len() >= PQ_TRAINING_SAMPLES {
                let training: Vec<Vec<f32>> =
                    buffer.iter().map(|(_, vector)| vector.clone()).collect();
                let num_centroids = 256usize.min(training.len().max(2));
                let trained = match ProductQuantizer::train(
                    &training,
                    auto_num_subspaces(point.vector.len()),
                    num_centroids,
                ) {
                    Ok(pq) => Some(pq),
                    Err(error) => {
                        // The training buffer is drained below regardless of
                        // outcome, so a failure here silently disables PQ for
                        // this batch (and every point already buffered) with
                        // no other signal — log it so it is observable.
                        tracing::warn!(%error, "PQ training failed; buffer discarded, quantizer stays unset");
                        None
                    }
                };
                #[cfg(feature = "persistence")]
                if let Some(ref pq) = trained {
                    if let Err(error) = pq.save_codebook(&self.storage.path) {
                        tracing::warn!(%error, "PQ codebook save failed; quantizer stays in-memory only");
                    }
                }
                *quantizer_guard = trained;
                backfill_samples = buffer.drain(..).collect();
            }
        }

        if let (Some(cache), Some(quantizer)) = (pq_cache, quantizer_guard.as_ref()) {
            for (id, vector) in backfill_samples {
                if let Ok(code) = quantizer.quantize(&vector) {
                    cache.insert(id, code);
                }
            }

            if let Ok(code) = quantizer.quantize(&point.vector) {
                cache.insert(point.id, code);
            }
        }
    }

    /// Updates all secondary indexes after an upsert (removes old values, inserts new ones).
    pub(crate) fn update_secondary_indexes_on_upsert(
        &self,
        id: u64,
        old_payload: Option<&serde_json::Value>,
        new_payload: Option<&serde_json::Value>,
    ) {
        let indexes = self.query.secondary_indexes.read();
        for (field, index) in indexes.iter() {
            if let Some(old_value) = old_payload
                .and_then(|p| p.get(field))
                .and_then(JsonValue::from_json)
            {
                self.remove_from_secondary_index(index, &old_value, id);
            }
            if let Some(new_value) = new_payload
                .and_then(|p| p.get(field))
                .and_then(JsonValue::from_json)
            {
                self.insert_into_secondary_index(index, new_value, id);
            }
        }
    }

    /// Removes entries from all secondary indexes for a deleted point.
    pub(crate) fn update_secondary_indexes_on_delete(
        &self,
        id: u64,
        old_payload: Option<&serde_json::Value>,
    ) {
        let Some(payload) = old_payload else {
            return;
        };
        let indexes = self.query.secondary_indexes.read();
        for (field, index) in indexes.iter() {
            if let Some(old_value) = payload.get(field).and_then(JsonValue::from_json) {
                self.remove_from_secondary_index(index, &old_value, id);
            }
        }
    }

    // These methods take `&self` for consistency with the impl block calling convention,
    // but the operations are logically index-directed and do not need instance state.
    #[allow(clippy::unused_self)]
    pub(crate) fn insert_into_secondary_index(
        &self,
        index: &SecondaryIndex,
        key: JsonValue,
        id: u64,
    ) {
        match index {
            SecondaryIndex::BTree(tree) => {
                let mut tree = tree.write();
                // Set semantics: no O(k) contains() scan per insert.
                tree.entry(key).or_default().insert(id);
            }
        }
    }

    #[allow(clippy::unused_self)]
    fn remove_from_secondary_index(&self, index: &SecondaryIndex, key: &JsonValue, id: u64) {
        match index {
            SecondaryIndex::BTree(tree) => {
                let mut tree = tree.write();
                if let Some(ids) = tree.get_mut(key) {
                    ids.remove(id);
                    if ids.is_empty() {
                        tree.remove(key);
                    }
                }
            }
        }
    }

    /// Replaces persisted histograms for a batch under last-writer-wins dedup.
    ///
    /// Builds the dedup map (`point_id -> index_of_last_occurrence`), keeps
    /// only the payload of the last occurrence for each id (zeroing the rest),
    /// and feeds the decrement/increment pair to `update_histograms_replace`
    /// in a single atomic read → modify → write cycle.
    ///
    /// Used by all upsert paths (`upsert`, `upsert_metadata`, `upsert_bulk`
    /// V2 and standard) to ensure:
    /// - Bug #47 — dedup by `point.id` so only the final payload counts;
    /// - Bug #49 — one histogram cycle instead of two (decrement then
    ///   increment happen together under `stats_io_mutex`).
    ///
    /// Issue #450 Phase 3.1: factored out of 4 identical call sites to shrink
    /// the duplicated surface in `collection/core/`.
    pub(super) fn apply_histogram_replace_dedup(
        &self,
        points: &[Point],
        old_payloads: &[Option<serde_json::Value>],
    ) {
        let dedup = Self::build_dedup_map(points);
        let new_payloads: Vec<Option<serde_json::Value>> = points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if dedup.get(&p.id) == Some(&i) {
                    p.payload.clone()
                } else {
                    None
                }
            })
            .collect();
        self.update_histograms_replace(old_payloads, &new_payloads);
    }
}
