//! CRUD operations for Collection (upsert, get, delete).
//!
//! Quantization caching helpers and secondary-index update helpers are in `crud_helpers.rs`.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::index::VectorIndex;
use crate::point::Point;
use crate::quantization::{BinaryQuantizedVector, PQVector, QuantizedVector, StorageMode};
use crate::storage::{LogPayloadStorage, PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;

use parking_lot::RwLockWriteGuard;
use std::collections::{BTreeMap, HashMap};

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
    /// Returns buffered sparse vectors for deferred insertion.
    fn upsert_storage_and_index(
        &self,
        points: &[Point],
        storage_mode: StorageMode,
    ) -> Result<Vec<(u64, BTreeMap<String, crate::index::sparse::SparseVector>)>> {
        let mut vector_storage = self.vector_storage.write();
        let mut payload_storage = self.payload_storage.write();
        let mut quant_guards = QuantizationGuards::acquire(self, storage_mode);

        let mut sparse_batch = Vec::new();
        for point in points {
            let old_payload = payload_storage.retrieve(point.id).ok().flatten();
            vector_storage.store(point.id, &point.vector)?;

            let (sq8, binary, pq) = (
                quant_guards.sq8.as_deref_mut(),
                quant_guards.binary.as_deref_mut(),
                quant_guards.pq.as_deref_mut(),
            );
            self.cache_quantized_vector(point, storage_mode, sq8, binary, pq);

            Self::store_or_delete_payload(&mut payload_storage, point)?;
            self.update_secondary_indexes_on_upsert(
                point.id,
                old_payload.as_ref(),
                point.payload.as_ref(),
            );
            self.insert_or_defer(point.id, &point.vector);
            Self::update_text_index(&self.text_index, point);
            Self::collect_sparse_vectors(point, &mut sparse_batch);
        }

        let point_count = vector_storage.len();
        vector_storage.flush()?;
        payload_storage.flush()?;
        drop(vector_storage);
        drop(payload_storage);

        self.config.write().point_count = point_count;
        self.maybe_merge_deferred();

        Ok(sparse_batch)
    }

    fn store_or_delete_payload(
        payload_storage: &mut LogPayloadStorage,
        point: &Point,
    ) -> Result<()> {
        if let Some(payload) = &point.payload {
            payload_storage.store(point.id, payload)?;
        } else {
            let _ = payload_storage.delete(point.id);
        }
        Ok(())
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
    fn update_text_index(text_index: &crate::index::Bm25Index, point: &Point) {
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
    fn invalidate_caches_and_bump_generation(&self) {
        *self.cached_stats.lock() = None;
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Inserts into HNSW directly, or buffers in the deferred indexer.
    ///
    /// When a deferred indexer is present, the vector is pushed into the
    /// deferred buffer instead of the HNSW graph. Otherwise falls through
    /// to `VectorIndex::insert`.
    ///
    /// Invariant: `self.deferred_indexer` is `Some` only when enabled
    /// (`build_deferred_indexer` filters on `cfg.enabled`), so no
    /// redundant `is_enabled()` check is needed here.
    fn insert_or_defer(&self, id: u64, vector: &[f32]) {
        #[cfg(feature = "persistence")]
        if let Some(ref di) = self.deferred_indexer {
            di.push(id, vector.to_vec());
            return;
        }
        self.index.insert(id, vector);
    }

    /// Triggers a deferred merge if the buffer has reached threshold.
    ///
    /// Drains buffered vectors and batch-inserts them into HNSW.
    /// No-op when deferred indexing is not configured.
    fn maybe_merge_deferred(&self) {
        #[cfg(feature = "persistence")]
        if let Some(ref di) = self.deferred_indexer {
            if di.should_merge() {
                self.merge_deferred_batch(di);
            }
        }
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

        let mut payload_storage = self.payload_storage.write();

        for point in &points {
            let old_payload = payload_storage.retrieve(point.id).ok().flatten();
            Self::store_or_delete_payload(&mut payload_storage, point)?;
            Self::update_text_index(&self.text_index, point);
            self.update_secondary_indexes_on_upsert(
                point.id,
                old_payload.as_ref(),
                point.payload.as_ref(),
            );
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

    /// Bulk insert optimized for high-throughput import.
    ///
    /// # Performance
    ///
    /// This method is optimized for bulk loading:
    /// - Uses parallel HNSW insertion (rayon)
    /// - Single flush at the end (not per-point)
    /// - No HNSW index save (deferred for performance)
    /// - ~15x faster than previous sequential approach on large batches (5000+)
    /// - Benchmark: 25-30 Kvec/s on 768D vectors
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert_bulk(&self, points: &[Point]) -> Result<usize> {
        if points.is_empty() {
            return Ok(0);
        }

        let dimension = self.config.read().dimension;
        for point in points {
            validate_dimension_match(dimension, point.dimension())?;
        }

        let vector_refs: Vec<(u64, &[f32])> =
            points.iter().map(|p| (p.id, p.vector.as_slice())).collect();
        let sparse_batch = Self::collect_sparse_batch(points);

        self.bulk_store_vectors(&vector_refs)?;
        self.bulk_store_payloads(points)?;

        let inserted = self.bulk_index_or_defer(vector_refs);
        self.config.write().point_count = self.vector_storage.read().len();

        self.apply_sparse_batch_bulk(&sparse_batch)?;
        self.invalidate_caches_and_bump_generation();

        Ok(inserted)
    }

    /// Batch-inserts into HNSW or defers into the deferred indexer.
    ///
    /// Returns the number of vectors processed (whether indexed directly
    /// or deferred for later merge).
    ///
    /// Invariant: `self.deferred_indexer` is `Some` only when enabled
    /// (`build_deferred_indexer` filters on `cfg.enabled`), so no
    /// redundant `is_enabled()` check is needed here.
    fn bulk_index_or_defer(&self, vector_refs: Vec<(u64, &[f32])>) -> usize {
        #[cfg(feature = "persistence")]
        if let Some(ref di) = self.deferred_indexer {
            di.extend(vector_refs.iter().map(|(id, v)| (*id, v.to_vec())));
            if di.should_merge() {
                self.merge_deferred_batch(di);
            }
            return vector_refs.len();
        }
        let inserted = self.index.insert_batch_parallel(vector_refs);
        self.index.set_searching_mode();
        inserted
    }

    /// Collects sparse vectors grouped by index name for batch insert.
    fn collect_sparse_batch(
        points: &[Point],
    ) -> BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>> {
        let mut batch: BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>> =
            BTreeMap::new();
        for point in points {
            if let Some(sv_map) = &point.sparse_vectors {
                for (name, sv) in sv_map {
                    batch
                        .entry(name.clone())
                        .or_default()
                        .push((point.id, sv.clone()));
                }
            }
        }
        batch
    }

    /// Stores vectors in bulk via batch WAL + mmap write.
    fn bulk_store_vectors(&self, vectors: &[(u64, &[f32])]) -> Result<()> {
        let mut storage = self.vector_storage.write();
        storage.store_batch(vectors)?;
        storage.flush()?;
        Ok(())
    }

    /// Stores payloads and updates BM25 text index in bulk.
    ///
    /// Uses `LogPayloadStorage::store_batch()` for a single WAL sync instead
    /// of per-point fsync, improving bulk insert throughput by 10-50x.
    fn bulk_store_payloads(&self, points: &[Point]) -> Result<()> {
        let entries: Vec<(u64, &serde_json::Value)> = points
            .iter()
            .filter_map(|p| p.payload.as_ref().map(|pl| (p.id, pl)))
            .collect();

        self.payload_storage.write().store_batch(&entries)?;

        for point in points {
            Self::update_text_index(&self.text_index, point);
        }

        Ok(())
    }

    /// Applies sparse batch with WAL-before-apply for bulk insert.
    fn apply_sparse_batch_bulk(
        &self,
        sparse_batch: &BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>>,
    ) -> Result<()> {
        if sparse_batch.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "persistence")]
        {
            for (name, docs) in sparse_batch {
                let wal_path =
                    crate::index::sparse::persistence::wal_path_for_name(&self.path, name);
                for (point_id, sv) in docs {
                    crate::index::sparse::persistence::wal_append_upsert(&wal_path, *point_id, sv)?;
                }
            }
        }
        let mut indexes = self.sparse_indexes.write();
        for (name, docs) in sparse_batch {
            let idx = indexes.entry(name.clone()).or_default();
            idx.insert_batch_chunk(docs);
        }
        Ok(())
    }

    /// Retrieves points by their IDs.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        let config = self.config.read();
        let is_metadata_only = config.metadata_only;
        drop(config);

        let payload_storage = self.payload_storage.read();

        if is_metadata_only {
            // For metadata-only collections, only retrieve payload
            ids.iter()
                .map(|&id| {
                    let payload = payload_storage.retrieve(id).ok().flatten()?;
                    Some(Point {
                        id,
                        vector: Vec::new(),
                        payload: Some(payload),
                        sparse_vectors: None,
                    })
                })
                .collect()
        } else {
            // For vector collections, retrieve both vector and payload
            let vector_storage = self.vector_storage.read();
            ids.iter()
                .map(|&id| {
                    let vector = vector_storage.retrieve(id).ok().flatten()?;
                    let payload = payload_storage.retrieve(id).ok().flatten();
                    Some(Point {
                        id,
                        vector,
                        payload,
                        sparse_vectors: None,
                    })
                })
                .collect()
        }
    }

    /// Deletes points by their IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        if self.config.read().metadata_only {
            self.delete_metadata_only(ids)?;
        } else {
            self.delete_vector_points(ids)?;
        }
        self.invalidate_caches_and_bump_generation();
        Ok(())
    }

    /// Deletes metadata-only points.
    fn delete_metadata_only(&self, ids: &[u64]) -> Result<()> {
        let mut payload_storage = self.payload_storage.write();
        for &id in ids {
            let old_payload = payload_storage.retrieve(id).ok().flatten();
            payload_storage.delete(id)?;
            self.text_index.remove_document(id);
            self.update_secondary_indexes_on_delete(id, old_payload.as_ref());
        }
        let point_count = payload_storage.ids().len();
        drop(payload_storage);
        self.config.write().point_count = point_count;
        Ok(())
    }

    /// Deletes vector points from all stores (vector, payload, index, caches, sparse, delta).
    fn delete_vector_points(&self, ids: &[u64]) -> Result<()> {
        let mut payload_storage = self.payload_storage.write();
        let mut vector_storage = self.vector_storage.write();
        let mut sq8_cache = self.sq8_cache.write();
        let mut binary_cache = self.binary_cache.write();
        let mut pq_cache = self.pq_cache.write();

        for &id in ids {
            let old_payload = payload_storage.retrieve(id).ok().flatten();
            vector_storage.delete(id)?;
            payload_storage.delete(id)?;
            self.index.remove(id);
            sq8_cache.remove(&id);
            binary_cache.remove(&id);
            pq_cache.remove(&id);
            self.text_index.remove_document(id);
            self.update_secondary_indexes_on_delete(id, old_payload.as_ref());
        }

        let point_count = vector_storage.len();
        drop(vector_storage);
        drop(payload_storage);
        drop(sq8_cache);
        drop(binary_cache);
        drop(pq_cache);
        self.config.write().point_count = point_count;

        self.delete_from_sparse_indexes(ids)?;

        // Lock order: delta_buffer(10) acquired after sparse_indexes(9) released.
        #[cfg(feature = "persistence")]
        for &id in ids {
            self.delta_buffer.remove(id);
        }

        // Lock order: deferred_indexer(11) acquired after delta_buffer(10).
        #[cfg(feature = "persistence")]
        if let Some(ref di) = self.deferred_indexer {
            for &id in ids {
                di.remove(id);
            }
        }

        Ok(())
    }

    /// Deletes IDs from sparse indexes with WAL-before-apply.
    fn delete_from_sparse_indexes(&self, ids: &[u64]) -> Result<()> {
        #[cfg(feature = "persistence")]
        {
            let indexes = self.sparse_indexes.read();
            for (name, _) in indexes.iter() {
                let wal_path =
                    crate::index::sparse::persistence::wal_path_for_name(&self.path, name);
                for &id in ids {
                    crate::index::sparse::persistence::wal_append_delete(&wal_path, id)?;
                }
            }
        }
        let indexes = self.sparse_indexes.read();
        for idx in indexes.values() {
            for &id in ids {
                idx.delete(id);
            }
        }
        Ok(())
    }

    /// Returns the number of points in the collection.
    /// Perf: Uses cached `point_count` from config instead of acquiring storage lock
    #[must_use]
    pub fn len(&self) -> usize {
        self.config.read().point_count
    }

    /// Returns true if the collection is empty.
    /// Perf: Uses cached `point_count` from config instead of acquiring storage lock
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.config.read().point_count == 0
    }

    /// Returns all point IDs in the collection.
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.payload_storage.read().ids()
    }
}
