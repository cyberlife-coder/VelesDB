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
type DedupMap = HashMap<u64, usize>;

struct QuantizationGuards<'a> {
    sq8: Option<RwLockWriteGuard<'a, HashMap<u64, QuantizedVector>>>,
    binary: Option<RwLockWriteGuard<'a, HashMap<u64, BinaryQuantizedVector>>>,
    pq: Option<RwLockWriteGuard<'a, HashMap<u64, PQVector>>>,
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

        let sparse_batch = self.upsert_storage_and_index(&points, storage_mode)?;

        self.apply_sparse_batch_upsert(&sparse_batch)?;
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
    fn upsert_storage_and_index(
        &self,
        points: &[Point],
        storage_mode: StorageMode,
    ) -> Result<Vec<(u64, BTreeMap<String, crate::index::sparse::SparseVector>)>> {
        // Phase 1: Batch storage under write locks (1 fsync per storage)
        let old_payloads = self.batch_store_all(points)?;

        // Phase 2: Per-point updates (no storage locks held)
        let sparse_batch = self.per_point_updates(points, &old_payloads, storage_mode);

        // Phase 3: Batch HNSW insert
        let vector_refs: Vec<(u64, &[f32])> =
            points.iter().map(|p| (p.id, p.vector.as_slice())).collect();
        self.bulk_index_or_defer(vector_refs);

        Ok(sparse_batch)
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
    fn collect_old_payloads(
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
    fn build_dedup_map(points: &[Point]) -> DedupMap {
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

    /// Checks whether label index updates are needed for this batch.
    ///
    /// Returns `true` when either new or old payloads contain `_labels`,
    /// ensuring stale labels are removed when a point drops its labels.
    fn needs_label_updates(points: &[Point], old_payloads: &[Option<serde_json::Value>]) -> bool {
        Self::any_point_has_labels(points)
            || old_payloads
                .iter()
                .any(|opt| opt.as_ref().is_some_and(|v| v.get("_labels").is_some()))
    }

    /// Pre-allocates the label update buffer when needed.
    fn alloc_label_buffer(
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
    fn resolve_effective_old<'a>(
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
    ///
    /// Skipped when parallel quantization already handled SQ8/Binary.
    /// PQ always runs sequentially (shared training state).
    fn maybe_quantize(
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
    ///
    /// LOCK ORDER: label_index(7) — after secondary_indexes(6), before edge_store(8).
    fn apply_label_updates(
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
    ///
    /// Returns `true` if quantization was handled (caller should skip per-point
    /// quantization in the main loop). Returns `false` for PQ, Full, or RaBitQ
    /// modes (which need the original sequential path).
    ///
    /// Issue #486: `from_f32()` is a pure function for SQ8 and Binary, so
    /// computation can run in parallel via rayon. Only the final cache
    /// insertion requires a write lock, held briefly for batch insert.
    fn try_parallel_quantize(&self, points: &[Point], storage_mode: StorageMode) -> bool {
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

    fn collect_sparse_vectors(
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

    /// Applies buffered sparse vector upserts with WAL-before-apply semantics.
    fn apply_sparse_batch_upsert(
        &self,
        sparse_batch: &[(u64, BTreeMap<String, crate::index::sparse::SparseVector>)],
    ) -> Result<()> {
        if sparse_batch.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "persistence")]
        {
            for (point_id, sv_map) in sparse_batch {
                for (name, sv) in sv_map {
                    let wal_path =
                        crate::index::sparse::persistence::wal_path_for_name(&self.path, name);
                    crate::index::sparse::persistence::wal_append_upsert(&wal_path, *point_id, sv)?;
                }
            }
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
    ///
    /// Filters out IDs that have been deleted from vector storage since they
    /// were buffered, preventing ghost vectors from being re-inserted into
    /// HNSW after a concurrent delete.
    ///
    /// Logs a warning if fewer vectors were inserted than expected, which
    /// indicates a partial failure (e.g., duplicate IDs filtered out,
    /// ghost-vector filtering, or graph insertion error). The drained
    /// vectors are not retried.
    #[cfg(feature = "persistence")]
    fn merge_deferred_batch(&self, di: &crate::collection::streaming::DeferredIndexer) {
        let drained = di.swap_and_drain();
        if drained.is_empty() {
            return;
        }
        // Filter out vectors deleted from storage during the buffer's
        // lifetime to prevent ghost re-insertion into HNSW.
        let storage = self.vector_storage.read();
        let valid: Vec<(u64, &[f32])> = drained
            .iter()
            .filter(|(id, _)| storage.retrieve(*id).ok().flatten().is_some())
            .map(|(id, v)| (*id, v.as_slice()))
            .collect();
        drop(storage); // Release read lock before batch insert
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
    ///
    /// Returns the number of vectors processed (whether indexed directly
    /// or deferred for later merge).
    ///
    /// Since v1.7.2, both `upsert()` and `upsert_bulk()` route through this
    /// method. The direct path calls `insert_batch_parallel` (rayon), which
    /// yields non-deterministic HNSW graph topology across runs. Search
    /// correctness and recall are unaffected.
    ///
    /// Invariant: `self.deferred_indexer` is `Some` only when enabled
    /// (`build_deferred_indexer` filters on `cfg.enabled`), so no
    /// redundant `is_enabled()` check is needed here.
    pub(super) fn bulk_index_or_defer(&self, vector_refs: Vec<(u64, &[f32])>) -> usize {
        let count = vector_refs.len();
        #[cfg(feature = "persistence")]
        if let Some(ref di) = self.deferred_indexer {
            di.extend(vector_refs.iter().map(|(id, v)| (*id, v.to_vec())));
            if di.should_merge() {
                self.merge_deferred_batch(di);
            }
            // Issue #423 Component 3: Track inserts for periodic HNSW save.
            // Reason: count fits in u64 (vector batch size bounded by memory).
            #[allow(clippy::cast_possible_truncation)]
            self.inserts_since_last_hnsw_save
                .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
            return count;
        }
        let inserted = self.index.insert_batch_parallel(vector_refs);
        // NOTE: set_searching_mode() removed — it was a no-op on the native HNSW
        // backend but acquired inner.write() (exclusive lock), serializing against
        // rayon readers. Removing it eliminates 20+ unnecessary lock cycles for
        // multi-batch bulk imports (Issue #486).
        // Issue #423 Component 3: Track inserts for periodic HNSW save.
        // Reason: count fits in u64 (vector batch size bounded by memory).
        #[allow(clippy::cast_possible_truncation)]
        self.inserts_since_last_hnsw_save
            .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
        inserted
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

        // LOCK ORDER: flush while payload_storage(3) still held, then drop before acquiring config(1).
        let point_count = payload_storage.ids().len();
        payload_storage.flush()?;
        drop(payload_storage);

        // config(1) only — all higher-numbered locks released above.
        self.config.write().point_count = point_count;
        self.invalidate_caches_and_bump_generation();
        Ok(())
    }
}
