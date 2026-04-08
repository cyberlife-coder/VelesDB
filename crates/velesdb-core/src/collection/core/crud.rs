//! Upsert operations for Collection.
//!
//! Read and delete operations are in `crud_read_delete.rs`.
//! Bulk-specific methods (`upsert_bulk`, `upsert_bulk_from_raw`) are in `crud_bulk.rs`.
//! Quantization caching helpers and secondary-index update helpers are in `crud_helpers.rs`.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::point::Point;
use crate::quantization::{BinaryQuantizedVector, PQVector, QuantizedVector, StorageMode};
use crate::storage::{LogPayloadStorage, PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;

use parking_lot::RwLockWriteGuard;
use std::collections::{BTreeMap, HashMap};

/// Pre-computed last-writer-wins dedup map: `point_id -> index_of_last_occurrence`.
///
/// Built once in `batch_store_all` and shared by both `write_deduped_payloads`
/// and `write_deduped_vectors` to avoid redundant map construction (Issue #425).
pub(super) type DedupMap = HashMap<u64, usize>;

pub(super) struct QuantizationGuards<'a> {
    pub(super) sq8: Option<RwLockWriteGuard<'a, HashMap<u64, QuantizedVector>>>,
    pub(super) binary: Option<RwLockWriteGuard<'a, HashMap<u64, BinaryQuantizedVector>>>,
    pub(super) pq: Option<RwLockWriteGuard<'a, HashMap<u64, PQVector>>>,
}

impl<'a> QuantizationGuards<'a> {
    fn acquire(collection: &'a Collection, mode: StorageMode) -> Self {
        Self {
            sq8: matches!(mode, StorageMode::SQ8).then(|| collection.sq8_cache.write()),
            binary: matches!(mode, StorageMode::Binary).then(|| collection.binary_cache.write()),
            pq: matches!(mode, StorageMode::ProductQuantization)
                .then(|| collection.pq_cache.write()),
        }
    }

    /// Acquires only the PQ cache guard (for when SQ8/Binary were handled in parallel).
    ///
    /// Issue #486: After parallel quantization for SQ8/Binary, only PQ mode
    /// still needs a guard for sequential processing.
    fn acquire_pq_only(collection: &'a Collection, mode: StorageMode) -> Self {
        Self {
            sq8: None,
            binary: None,
            pq: matches!(mode, StorageMode::ProductQuantization)
                .then(|| collection.pq_cache.write()),
        }
    }
}

impl Collection {
    /// Inserts or updates points in the collection.
    ///
    /// Accepts any iterator of points (Vec, slice, array, etc.)
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension, or if
    /// attempting to insert vectors into a metadata-only collection.
    pub fn upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        let points: Vec<Point> = points.into_iter().collect();
        let config = self.config.read();
        let dimension = config.dimension;
        let storage_mode = config.storage_mode;

        if config.metadata_only {
            for point in &points {
                if !point.vector.is_empty() {
                    return Err(Error::VectorNotAllowed(config.name.clone()));
                }
            }
            drop(config);
            return self.upsert_metadata(points);
        }
        drop(config);

        for point in &points {
            validate_dimension_match(dimension, point.dimension())?;
        }

        let (sparse_batch, old_payloads) = self.upsert_storage_and_index(&points, storage_mode)?;

        self.apply_sparse_batch_upsert(&sparse_batch)?;

        // Incremental histogram maintenance: decrement old values and
        // increment new values in a single atomic read → modify → write
        // cycle (Bug #49: avoids 2× I/O of separate delete + upsert calls).
        // Only the last occurrence per ID is counted for new payloads
        // (Bug #47: dedup to match last-writer-wins storage semantics).
        let dedup = Self::build_dedup_map(&points);
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
        self.update_histograms_replace(&old_payloads, &new_payloads);

        self.invalidate_caches_and_bump_generation();
        Ok(())
    }

    /// Stores vectors, payloads, and indexes for a batch of points.
    ///
    /// Three-phase pipeline to minimize lock contention and I/O:
    /// 1. Batch storage: `store_batch()` for vectors + payloads (1 fsync each)
    /// 2. Per-point updates: secondary indexes, quantization, text, sparse
    /// 3. Batch HNSW insert via `bulk_index_or_defer()`
    ///
    /// # Crash Recovery
    ///
    /// A crash between Phase 1 and Phase 3 leaves vectors durably stored but
    /// absent from the HNSW index. On the next `Collection::open()`, gap
    /// detection compares storage IDs against HNSW mappings and re-indexes
    /// any missing vectors. The recovery window is bounded by one batch.
    ///
    /// Returns buffered sparse vectors for deferred insertion.
    #[allow(clippy::type_complexity)] // SAFETY: tuple of (sparse_batch, old_payloads) — extracting a named type adds indirection without clarity
    fn upsert_storage_and_index(
        &self,
        points: &[Point],
        storage_mode: StorageMode,
    ) -> Result<(
        Vec<(u64, BTreeMap<String, crate::index::sparse::SparseVector>)>,
        Vec<Option<serde_json::Value>>,
    )> {
        // Phase 1: Batch storage under write locks (1 fsync per storage)
        let old_payloads = self.batch_store_all(points)?;

        // Phase 2: Per-point updates (no storage locks held)
        let sparse_batch = self.per_point_updates(points, &old_payloads, storage_mode);

        // Phase 3: Batch HNSW insert
        let vector_refs: Vec<(u64, &[f32])> =
            points.iter().map(|p| (p.id, p.vector.as_slice())).collect();
        self.bulk_index_or_defer(vector_refs);

        Ok((sparse_batch, old_payloads))
    }

    /// Phase 1: Batch-stores vectors and payloads with minimal lock scope.
    ///
    /// Pre-collects old payloads (needed for secondary index updates),
    /// then writes all vectors and payloads in single batch calls (1 fsync each).
    ///
    /// Deduplicates intra-batch duplicate IDs using last-writer-wins semantics:
    /// only the final occurrence per ID is written to the WAL, avoiding wasteful
    /// intermediate entries that would bloat the log and slow replay.
    ///
    /// After this method returns, vectors and payloads are durable on disk.
    /// A crash before Phase 3 (HNSW insertion) is recovered by gap detection
    /// on the next `Collection::open()`.
    ///
    /// # Parallel I/O (Issue #424)
    ///
    /// With the `persistence` feature (which enables `rayon`), payload and
    /// vector writes run concurrently via `rayon::join` after old-payload
    /// collection completes. This is safe because:
    ///
    /// - Payload and vector storage use independent `RwLock`s (positions 3
    ///   and 2 in the lock order). Neither closure acquires both locks.
    /// - Crash recovery only requires that both are durable before Phase 3
    ///   (HNSW insertion). There is no ordering dependency between payload
    ///   and vector WAL writes — gap detection on `Collection::open()` handles
    ///   any partial write scenario.
    /// - `old_payloads` collection is completed and the payload lock is
    ///   released before the fork, so both closures start from clean state.
    /// - The TOCTOU gap between old-payload collection and the parallel
    ///   write is acceptable: `old_payloads` feeds Phase 2 secondary-index
    ///   updates, and each concurrent batch tracks its own `seen_payloads`.
    ///
    /// Returns the old payloads for Phase 2.
    fn batch_store_all(&self, points: &[Point]) -> Result<Vec<Option<serde_json::Value>>> {
        // Collect old payloads under the payload write lock, then release.
        // The write lock prevents concurrent payload mutations during the read.
        let old_payloads = {
            let payload_storage = self.payload_storage.write();
            let result = Self::collect_old_payloads(points, &payload_storage);
            drop(payload_storage);
            result
        };

        // Issue #425: Build the dedup map once and share it across both
        // write paths, avoiding redundant HashMap construction.
        let dedup_map = Self::build_dedup_map(points);

        // Issue #424: Parallel I/O — payload and vector writes are independent
        // after old_payloads collection. Run them concurrently via rayon::join.
        // rayon is gated on the persistence feature.
        #[cfg(feature = "persistence")]
        {
            let (payload_result, vector_result) = rayon::join(
                || self.write_and_flush_payloads(points, &dedup_map),
                || self.write_deduped_vectors(points, &dedup_map),
            );
            payload_result?;
            vector_result?;
        }

        #[cfg(not(feature = "persistence"))]
        {
            self.write_and_flush_payloads(points, &dedup_map)?;
            self.write_deduped_vectors(points, &dedup_map)?;
        }

        Ok(old_payloads)
    }

    /// Writes deduped payloads and flushes the storage.
    ///
    /// Issue #424: Extracted so it can be called from `rayon::join` in the
    /// parallel I/O path. Acquires the `payload_storage` write lock internally.
    ///
    /// Issue #425: Accepts a pre-computed `dedup_map` to avoid rebuilding
    /// the last-writer-wins map redundantly.
    fn write_and_flush_payloads(&self, points: &[Point], dedup_map: &DedupMap) -> Result<()> {
        let mut payload_storage = self.payload_storage.write();
        Self::write_deduped_payloads(points, &mut payload_storage, dedup_map)?;
        payload_storage.flush()?;
        Ok(())
    }

    /// Retrieves pre-batch payloads, querying storage only once per unique ID.
    ///
    /// For intra-batch duplicates, only the first occurrence needs the pre-batch
    /// value; subsequent occurrences are handled by `seen_payloads` in Phase 2.
    pub(crate) fn collect_old_payloads(
        points: &[Point],
        storage: &LogPayloadStorage,
    ) -> Vec<Option<serde_json::Value>> {
        let mut seen = std::collections::HashSet::new();
        points
            .iter()
            .map(|p| {
                if seen.insert(p.id) {
                    // First occurrence — retrieve pre-batch payload from storage
                    storage.retrieve(p.id).ok().flatten()
                } else {
                    None // Duplicate — Phase 2 uses seen_payloads instead
                }
            })
            .collect()
    }

    /// Builds a last-writer-wins dedup map: `point_id -> index_of_last_occurrence`.
    ///
    /// Issue #425: Computed once in `batch_store_all` and shared by both
    /// `write_deduped_payloads` and `write_deduped_vectors` to avoid
    /// redundant `HashMap` construction.
    pub(super) fn build_dedup_map(points: &[Point]) -> DedupMap {
        let mut map = HashMap::with_capacity(points.len());
        for (i, p) in points.iter().enumerate() {
            map.insert(p.id, i);
        }
        map
    }

    /// Writes only the last payload per ID to the WAL, then deletes IDs whose
    /// final occurrence has `payload=None`.
    ///
    /// Issue #425: Accepts a pre-computed `dedup_map` instead of building
    /// its own, consolidating the two redundant maps into one.
    fn write_deduped_payloads(
        points: &[Point],
        storage: &mut LogPayloadStorage,
        dedup_map: &DedupMap,
    ) -> Result<()> {
        // Only write the final payload per ID (skip intermediate duplicates)
        let deduped: Vec<(u64, &serde_json::Value)> = points
            .iter()
            .enumerate()
            .filter(|&(i, p)| dedup_map.get(&p.id) == Some(&i) && p.payload.is_some())
            .filter_map(|(_, p)| p.payload.as_ref().map(|pl| (p.id, pl)))
            .collect();
        storage.store_batch(&deduped)?;

        // Delete IDs whose final occurrence has payload=None
        for (i, p) in points.iter().enumerate() {
            if dedup_map.get(&p.id) == Some(&i) && p.payload.is_none() {
                let _ = storage.delete(p.id);
            }
        }
        Ok(())
    }

    /// Writes only the last vector per ID to vector storage.
    ///
    /// Issue #425: Accepts a pre-computed `dedup_map` instead of building
    /// its own, consolidating the two redundant maps into one.
    fn write_deduped_vectors(&self, points: &[Point], dedup_map: &DedupMap) -> Result<()> {
        let deduped: Vec<(u64, &[f32])> = points
            .iter()
            .enumerate()
            .filter(|&(i, p)| dedup_map.get(&p.id) == Some(&i))
            .map(|(_, p)| (p.id, p.vector.as_slice()))
            .collect();

        let mut vector_storage = self.vector_storage.write();
        vector_storage.store_batch(&deduped)?;
        let point_count = vector_storage.len();
        vector_storage.flush()?;
        drop(vector_storage);

        self.config.write().point_count = point_count;
        Ok(())
    }

    /// Returns `true` when Phase 2 processing can be skipped entirely.
    ///
    /// Issue #425: For the common case (`StorageMode::Full`, no secondary
    /// indexes, empty BM25 index, no sparse vectors in the batch), Phase 2
    /// does zero useful work. Skipping avoids `QuantizationGuards` acquisition,
    /// `seen_payloads` HashMap allocation, and the per-point loop.
    fn can_skip_phase2(&self, points: &[Point], storage_mode: StorageMode) -> bool {
        // Quantization caching is a no-op only for Full and RaBitQ modes
        let no_quantization = matches!(storage_mode, StorageMode::Full | StorageMode::RaBitQ);
        if !no_quantization {
            return false;
        }

        // Secondary indexes require per-point old/new payload diffing
        let no_secondary = self.secondary_indexes.read().is_empty();
        if !no_secondary {
            return false;
        }

        // BM25 text index: skip only when the index is empty AND no point
        // carries a payload (nothing to add, nothing to remove)
        let bm25_empty = self.text_index.is_empty();
        let any_payload = points.iter().any(|p| p.payload.is_some());
        if !bm25_empty || any_payload {
            return false;
        }

        // Label index: when populated, old payloads may contain `_labels`
        // that need cleanup. Phase 2 must run to call `apply_label_updates`.
        if !self.label_index.read().is_empty() {
            return false;
        }

        // Sparse vectors require collection into the sparse batch buffer
        let any_sparse = points.iter().any(Point::has_sparse_vectors);
        !any_sparse
    }

    /// Phase 2: Per-point updates that don't need storage write locks.
    ///
    /// Tracks the effective "old payload" per ID to handle within-batch
    /// duplicates correctly: when id=5 appears twice, the second occurrence
    /// sees the first occurrence's payload as its "old" (not the pre-batch
    /// original), ensuring secondary indexes stay consistent.
    ///
    /// Issue #425: Fast-path skips the entire loop when no secondary
    /// processing is needed (see `can_skip_phase2`).
    ///
    /// Issue #486: For SQ8/Binary modes with rayon, quantization runs in
    /// parallel before the main loop, avoiding the per-point lock overhead.
    fn per_point_updates(
        &self,
        points: &[Point],
        old_payloads: &[Option<serde_json::Value>],
        storage_mode: StorageMode,
    ) -> Vec<(u64, BTreeMap<String, crate::index::sparse::SparseVector>)> {
        // Issue #425: Fast-path — skip Phase 2 entirely when no secondary
        // processing is needed. Avoids lock acquisition, HashMap allocation,
        // and the per-point loop for the common StorageMode::Full case.
        if self.can_skip_phase2(points, storage_mode) {
            return Vec::new();
        }

        // Issue #486: Parallel quantization for SQ8/Binary — compute all
        // quantized vectors via rayon, then batch-insert under a single
        // write lock. PQ mode is handled sequentially (shared training state).
        let quant_done = self.try_parallel_quantize(points, storage_mode);

        let mut quant_guards = if quant_done {
            // Quantization already applied — no guards needed for SQ8/Binary
            QuantizationGuards::acquire_pq_only(self, storage_mode)
        } else {
            QuantizationGuards::acquire(self, storage_mode)
        };

        self.per_point_sequential_updates(
            points,
            old_payloads,
            storage_mode,
            &mut quant_guards,
            quant_done,
        )
    }

    /// Runs the sequential per-point loop for secondary indexes, BM25, sparse
    /// vectors, labels, and (when not pre-computed) quantization.
    ///
    /// Extracted from `per_point_updates` to keep each function under 50 NLOC.
    fn per_point_sequential_updates(
        &self,
        points: &[Point],
        old_payloads: &[Option<serde_json::Value>],
        storage_mode: StorageMode,
        quant_guards: &mut QuantizationGuards<'_>,
        quant_done: bool,
    ) -> Vec<(u64, BTreeMap<String, crate::index::sparse::SparseVector>)> {
        let mut sparse_batch = Vec::new();
        let mut seen_payloads: HashMap<u64, Option<&serde_json::Value>> = HashMap::new();
        let skip_bm25 = self.text_index.is_empty() && !points.iter().any(|p| p.payload.is_some());
        let needs_label_updates = Self::needs_label_updates(points, old_payloads);
        let mut label_updates = Self::alloc_label_buffer(needs_label_updates, points.len());

        for (point, pre_batch_old) in points.iter().zip(old_payloads) {
            let effective_old =
                Self::resolve_effective_old(&seen_payloads, point.id, pre_batch_old.as_ref());
            Self::maybe_quantize(self, point, storage_mode, quant_guards, quant_done);
            self.update_secondary_indexes_on_upsert(
                point.id,
                effective_old,
                point.payload.as_ref(),
            );
            if !skip_bm25 {
                Self::update_text_index(&self.text_index, point);
            }
            Self::collect_sparse_vectors(point, &mut sparse_batch);
            if needs_label_updates {
                label_updates.push((point.id, effective_old.cloned(), point.payload.clone()));
            }
            seen_payloads.insert(point.id, point.payload.as_ref());
        }

        Self::apply_label_updates(&self.label_index, &label_updates);
        sparse_batch
    }

    /// Inserts or updates metadata-only points (no vectors).
    ///
    /// This method is for metadata-only collections. Points should have
    /// empty vectors and only contain payload data.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn upsert_metadata(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        let points: Vec<Point> = points.into_iter().collect();

        // LOCK ORDER: payload_storage(3) → label_index(7).
        let mut payload_storage = self.payload_storage.write();
        let mut label_idx = self.label_index.write();

        // Collect old payloads for histogram decrements before they are overwritten.
        // Bug #46: use collect_old_payloads to deduplicate by ID — only the
        // first occurrence retrieves the pre-batch value; duplicates get None
        // so the old value is decremented exactly once.
        let old_payloads_for_hist = Self::collect_old_payloads(&points, &payload_storage);

        for point in &points {
            let old_payload = payload_storage.retrieve(point.id).ok().flatten();
            if let Some(payload) = &point.payload {
                payload_storage.store(point.id, payload)?;
            } else {
                let _ = payload_storage.delete(point.id);
            }
            Self::update_text_index(&self.text_index, point);
            self.update_secondary_indexes_on_upsert(
                point.id,
                old_payload.as_ref(),
                point.payload.as_ref(),
            );

            // Maintain label index for _labels-bearing payloads.
            if let Some(ref old) = old_payload {
                label_idx.remove_from_payload(point.id, old);
            }
            if let Some(ref payload) = point.payload {
                label_idx.index_from_payload(point.id, payload);
            }
        }

        // LOCK ORDER: drop label_index(7) before acquiring config(1) and stats_io_mutex(12).
        drop(label_idx);

        // LOCK ORDER: flush while payload_storage(3) still held, then drop before acquiring config(1).
        let point_count = payload_storage.ids().len();
        payload_storage.flush()?;
        drop(payload_storage);

        // config(1) only — payload_storage(3) and label_index(7) both released above.
        self.config.write().point_count = point_count;

        // Incremental histogram maintenance for metadata-only collections:
        // decrement old values and increment new values in one atomic cycle.
        // Bug #47: only the last occurrence per ID is counted for new payloads
        // to match last-writer-wins storage semantics.
        let dedup = Self::build_dedup_map(&points);
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
        self.update_histograms_replace(&old_payloads_for_hist, &new_payloads);

        self.invalidate_caches_and_bump_generation();
        Ok(())
    }
}
