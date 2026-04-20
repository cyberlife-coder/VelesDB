//! Label, sparse-vector, and deferred-indexing helpers for CRUD operations.
//!
//! Extracted from `crud.rs` to reduce NLOC.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::Point;
use crate::quantization::StorageMode;
use crate::storage::VectorStorage;
use std::collections::{BTreeMap, HashMap};

use super::crud_helpers::QuantizationGuards;

impl Collection {
    /// Checks whether label index updates are needed for this batch.
    pub(super) fn needs_label_updates(
        points: &[Point],
        old_payloads: &[Option<serde_json::Value>],
    ) -> bool {
        Self::any_point_has_labels(points)
            || old_payloads
                .iter()
                .any(|opt| opt.as_ref().is_some_and(|v| v.get("_labels").is_some()))
    }

    /// Pre-allocates the label update buffer when needed.
    pub(super) fn alloc_label_buffer(
        needed: bool,
        capacity: usize,
    ) -> Vec<(u64, Option<serde_json::Value>, Option<serde_json::Value>)> {
        if needed {
            Vec::with_capacity(capacity)
        } else {
            Vec::new()
        }
    }

    /// Returns `true` if any point carries `_labels` in its payload.
    pub(super) fn any_point_has_labels(points: &[Point]) -> bool {
        points.iter().any(|p| {
            p.payload
                .as_ref()
                .is_some_and(|v| v.get("_labels").is_some())
        })
    }

    /// Resolves the effective "old payload" for a point, accounting for
    /// within-batch duplicate IDs.
    pub(super) fn resolve_effective_old<'a>(
        seen: &HashMap<u64, Option<&'a serde_json::Value>>,
        id: u64,
        pre_batch_old: Option<&'a serde_json::Value>,
    ) -> Option<&'a serde_json::Value> {
        if let Some(&inner) = seen.get(&id) {
            inner
        } else {
            pre_batch_old
        }
    }

    /// Conditionally caches a quantized vector for a single point.
    pub(super) fn maybe_quantize(
        collection: &Collection,
        point: &Point,
        storage_mode: StorageMode,
        quant_guards: &mut QuantizationGuards<'_>,
        quant_done: bool,
    ) {
        if !quant_done {
            let (sq8, binary, pq) = (
                quant_guards.sq8.as_deref_mut(),
                quant_guards.binary.as_deref_mut(),
                quant_guards.pq.as_deref_mut(),
            );
            collection.cache_quantized_vector(point, storage_mode, sq8, binary, pq);
        } else if matches!(storage_mode, StorageMode::ProductQuantization) {
            let pq = quant_guards.pq.as_deref_mut();
            collection.cache_quantized_vector(point, storage_mode, None, None, pq);
        }
    }

    /// Applies buffered label index updates in a single write lock scope.
    pub(super) fn apply_label_updates(
        label_index: &parking_lot::RwLock<crate::collection::graph::LabelIndex>,
        label_updates: &[(u64, Option<serde_json::Value>, Option<serde_json::Value>)],
    ) {
        if label_updates.is_empty() {
            return;
        }
        let mut label_idx = label_index.write();
        for (id, old, new) in label_updates {
            if let Some(old_val) = old {
                label_idx.remove_from_payload(*id, old_val);
            }
            if let Some(new_val) = new {
                label_idx.index_from_payload(*id, new_val);
            }
        }
    }

    /// Attempts parallel quantization for SQ8/Binary modes.
    pub(super) fn try_parallel_quantize(
        &self,
        points: &[Point],
        storage_mode: StorageMode,
    ) -> bool {
        #[cfg(feature = "persistence")]
        match storage_mode {
            StorageMode::SQ8 => {
                self.batch_quantize_sq8_parallel(points);
                true
            }
            StorageMode::Binary => {
                self.batch_quantize_binary_parallel(points);
                true
            }
            _ => false,
        }
        #[cfg(not(feature = "persistence"))]
        {
            let _ = (points, storage_mode);
            false
        }
    }

    /// Collects sparse vectors from a point into the batch buffer.
    pub(super) fn collect_sparse_vectors(
        point: &Point,
        sparse_batch: &mut Vec<(u64, BTreeMap<String, crate::index::sparse::SparseVector>)>,
    ) {
        if let Some(sv_map) = &point.sparse_vectors {
            if !sv_map.is_empty() {
                sparse_batch.push((point.id, sv_map.clone()));
            }
        }
    }

    /// Updates the BM25 text index for a single point.
    pub(super) fn update_text_index(text_index: &crate::index::Bm25Index, point: &Point) {
        if let Some(payload) = &point.payload {
            let text = Self::extract_text_from_payload(payload);
            if !text.is_empty() {
                text_index.add_document(point.id, &text);
            }
        } else {
            text_index.remove_document(point.id);
        }
    }

    /// Appends `(name, point_id, sparse_vector)` triples to the per-index
    /// sparse WAL under WAL-before-apply semantics.
    ///
    /// Centralises the `wal_path_for_name` + `wal_append_upsert` loop that was
    /// duplicated between `apply_sparse_batch_upsert` (single-point path) and
    /// `apply_sparse_batch_bulk` (bulk path). Callers keep ownership of their
    /// input shape (`Vec<(u64, BTreeMap)>` vs `BTreeMap<String, Vec<(u64,
    /// SparseVector)>>`) and build the iterator of triples themselves, which
    /// keeps this helper allocation-free.
    ///
    /// Feature-gated on `persistence` — on targets without persistence the
    /// sparse WAL does not exist and the caller short-circuits.
    ///
    /// Issue #450 Phase 3.1.
    #[cfg(feature = "persistence")]
    pub(super) fn append_sparse_wal_entries<'a, I>(&self, entries: I) -> Result<()>
    where
        I: IntoIterator<Item = (&'a str, u64, &'a crate::index::sparse::SparseVector)>,
    {
        // Cache wal_path across consecutive entries sharing the same index name
        // so callers that yield entries grouped by name (e.g. apply_sparse_batch_bulk)
        // retain the O(N_NAMES) path-resolution cost of the pre-refactor code
        // rather than paying O(N_ENTRIES). Mixed-name callers degrade gracefully
        // to one resolution per entry, matching the original per-triple cost.
        let mut cached: Option<(&'a str, std::path::PathBuf)> = None;
        for (name, point_id, sv) in entries {
            if cached.as_ref().map(|(cached_name, _)| *cached_name) != Some(name) {
                let wal_path =
                    crate::index::sparse::persistence::wal_path_for_name(&self.path, name);
                cached = Some((name, wal_path));
            }
            let wal_path = &cached
                .as_ref()
                .expect("cache populated on first iteration")
                .1;
            crate::index::sparse::persistence::wal_append_upsert(wal_path, point_id, sv)?;
        }
        Ok(())
    }

    /// Applies buffered sparse vector upserts with WAL-before-apply semantics.
    pub(super) fn apply_sparse_batch_upsert(
        &self,
        sparse_batch: &[(u64, BTreeMap<String, crate::index::sparse::SparseVector>)],
    ) -> Result<()> {
        if sparse_batch.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "persistence")]
        {
            self.append_sparse_wal_entries(sparse_batch.iter().flat_map(|(point_id, sv_map)| {
                sv_map
                    .iter()
                    .map(move |(name, sv)| (name.as_str(), *point_id, sv))
            }))?;
        }
        let mut indexes = self.sparse_indexes.write();
        for (point_id, sv_map) in sparse_batch {
            for (name, sv) in sv_map {
                let idx = indexes.entry(name.clone()).or_default();
                idx.insert(*point_id, sv);
            }
        }
        Ok(())
    }

    /// Invalidates stats cache and bumps write generation.
    pub(super) fn invalidate_caches_and_bump_generation(&self) {
        *self.cached_stats.lock() = None;
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Drains the deferred indexer and batch-inserts into HNSW.
    #[cfg(feature = "persistence")]
    pub(super) fn merge_deferred_batch(&self, di: &crate::collection::streaming::DeferredIndexer) {
        let drained = di.swap_and_drain();
        if drained.is_empty() {
            return;
        }
        let storage = self.vector_storage.read();
        let valid: Vec<(u64, &[f32])> = drained
            .iter()
            .filter(|(id, _)| storage.retrieve(*id).ok().flatten().is_some())
            .map(|(id, v)| (*id, v.as_slice()))
            .collect();
        drop(storage);
        let expected = valid.len();
        if valid.is_empty() {
            return;
        }
        let inserted = self.index.insert_batch_parallel(valid);
        if inserted < expected {
            tracing::warn!("merge_deferred_batch: inserted {inserted}/{expected} vectors");
        }
    }

    /// Batch-inserts into HNSW or defers into the deferred indexer.
    pub(super) fn bulk_index_or_defer(&self, vector_refs: Vec<(u64, &[f32])>) -> usize {
        let count = vector_refs.len();
        #[cfg(feature = "persistence")]
        if let Some(ref di) = self.deferred_indexer {
            di.extend(vector_refs.iter().map(|(id, v)| (*id, v.to_vec())));
            if di.should_merge() {
                self.merge_deferred_batch(di);
            }
            #[allow(clippy::cast_possible_truncation)]
            self.inserts_since_last_hnsw_save
                .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
            return count;
        }
        let inserted = self.index.insert_batch_parallel(vector_refs);
        #[allow(clippy::cast_possible_truncation)]
        self.inserts_since_last_hnsw_save
            .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
        inserted
    }
}
