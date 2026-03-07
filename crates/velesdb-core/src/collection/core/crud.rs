//! CRUD operations for Collection (upsert, get, delete).
//!
//! Quantization caching helpers and secondary-index update helpers are in `crud_helpers.rs`.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::index::VectorIndex;
use crate::point::Point;
use crate::quantization::StorageMode;
use crate::storage::{PayloadStorage, VectorStorage};

use std::collections::BTreeMap;

impl Collection {
    /// Inserts or updates points in the collection.
    ///
    /// Accepts any iterator of points (Vec, slice, array, etc.)
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension, or if
    /// attempting to insert vectors into a metadata-only collection.
    #[allow(clippy::too_many_lines)] // Monolithic for coherent lock-ordering; refactor tracked separately.
    pub fn upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        let points: Vec<Point> = points.into_iter().collect();
        let config = self.config.read();
        let dimension = config.dimension;
        let storage_mode = config.storage_mode;
        let metadata_only = config.metadata_only;

        if metadata_only {
            for point in &points {
                if !point.vector.is_empty() {
                    // Lazy clone: name only allocated on this error path.
                    return Err(Error::VectorNotAllowed(config.name.clone()));
                }
            }
            drop(config);
            return self.upsert_metadata(points);
        }
        drop(config);

        for point in &points {
            if point.dimension() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: point.dimension(),
                });
            }
        }

        // Buffer sparse data for batch insert after storage locks are released.
        // LOCK ORDER: sparse_indexes(9) acquired AFTER vector_storage(2) + payload_storage(3).
        let mut sparse_batch: Vec<(u64, BTreeMap<String, crate::index::sparse::SparseVector>)> =
            Vec::new();

        let mut vector_storage = self.vector_storage.write();
        let mut payload_storage = self.payload_storage.write();

        let mut sq8_cache = match storage_mode {
            StorageMode::SQ8 => Some(self.sq8_cache.write()),
            _ => None,
        };
        let mut binary_cache = match storage_mode {
            StorageMode::Binary => Some(self.binary_cache.write()),
            _ => None,
        };
        let mut pq_cache = match storage_mode {
            StorageMode::ProductQuantization => Some(self.pq_cache.write()),
            _ => None,
        };

        for point in points {
            let old_payload = payload_storage.retrieve(point.id).ok().flatten();
            vector_storage
                .store(point.id, &point.vector)
                .map_err(Error::Io)?;

            self.cache_quantized_vector(
                &point,
                storage_mode,
                sq8_cache.as_deref_mut(),
                binary_cache.as_deref_mut(),
                pq_cache.as_deref_mut(),
            );

            if let Some(payload) = &point.payload {
                payload_storage
                    .store(point.id, payload)
                    .map_err(Error::Io)?;
            } else {
                let _ = payload_storage.delete(point.id);
            }

            self.update_secondary_indexes_on_upsert(
                point.id,
                old_payload.as_ref(),
                point.payload.as_ref(),
            );

            self.index.insert(point.id, &point.vector);

            if let Some(payload) = &point.payload {
                let text = Self::extract_text_from_payload(payload);
                if !text.is_empty() {
                    self.text_index.add_document(point.id, &text);
                }
            } else {
                self.text_index.remove_document(point.id);
            }

            // Buffer sparse vectors for batch insert after releasing storage locks.
            if let Some(sv_map) = point.sparse_vectors {
                if !sv_map.is_empty() {
                    sparse_batch.push((point.id, sv_map));
                }
            }
        }

        // LOCK ORDER: flush while vector_storage(2) + payload_storage(3) still held,
        // then drop both before acquiring config(1) alone to avoid inversion.
        let point_count = vector_storage.len();
        vector_storage.flush().map_err(Error::Io)?;
        payload_storage.flush().map_err(Error::Io)?;
        drop(vector_storage);
        drop(payload_storage);

        // config(1) only — all higher-numbered locks released above.
        let mut config = self.config.write();
        config.point_count = point_count;
        drop(config);

        self.index.save(&self.path).map_err(Error::Io)?;

        // LOCK ORDER: sparse_indexes(9) — acquired after all lower-numbered locks released.
        if !sparse_batch.is_empty() {
            // WAL-before-apply: persist the intent to disk BEFORE mutating the
            // in-memory index. A crash between WAL write and index insert is safe
            // because the WAL is replayed on recovery; a crash after index insert
            // but before WAL write would lose the update.
            #[cfg(feature = "persistence")]
            {
                for (point_id, sv_map) in &sparse_batch {
                    for (name, sv) in sv_map {
                        let wal_path =
                            crate::index::sparse::persistence::wal_path_for_name(&self.path, name);
                        crate::index::sparse::persistence::wal_append_upsert(
                            &wal_path, *point_id, sv,
                        )?;
                    }
                }
            }

            let mut indexes = self.sparse_indexes.write();
            for (point_id, sv_map) in &sparse_batch {
                for (name, sv) in sv_map {
                    let idx = indexes.entry(name.clone()).or_default();
                    idx.insert(*point_id, sv);
                }
            }
        }

        // Invalidate stats cache so the next get_stats() recomputes fresh data.
        *self.cached_stats.lock() = None;

        // Bump write generation once per batch (CACHE-01 invalidation counter).
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
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
            // Store Payload (metadata-only points must have payload)
            if let Some(payload) = &point.payload {
                payload_storage
                    .store(point.id, payload)
                    .map_err(Error::Io)?;

                // Update BM25 Text Index for full-text search
                let text = Self::extract_text_from_payload(payload);
                if !text.is_empty() {
                    self.text_index.add_document(point.id, &text);
                }
            } else {
                let _ = payload_storage.delete(point.id);
                self.text_index.remove_document(point.id);
            }

            self.update_secondary_indexes_on_upsert(
                point.id,
                old_payload.as_ref(),
                point.payload.as_ref(),
            );
        }

        // LOCK ORDER: flush while payload_storage(3) still held, then drop before acquiring config(1).
        let point_count = payload_storage.ids().len();
        payload_storage.flush().map_err(Error::Io)?;
        drop(payload_storage);

        // config(1) only — all higher-numbered locks released above.
        let mut config = self.config.write();
        config.point_count = point_count;
        drop(config);

        // Invalidate stats cache so the next get_stats() recomputes fresh data.
        *self.cached_stats.lock() = None;

        // Bump write generation once per batch (CACHE-01 invalidation counter).
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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

        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        for point in points {
            if point.dimension() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: point.dimension(),
                });
            }
        }

        // Perf: Collect vectors for parallel HNSW insertion (needed for clone anyway)
        let vectors_for_hnsw: Vec<(u64, Vec<f32>)> =
            points.iter().map(|p| (p.id, p.vector.clone())).collect();

        // Perf: Single batch WAL write + contiguous mmap write
        // Use references from vectors_for_hnsw to avoid double allocation
        let vectors_for_storage: Vec<(u64, &[f32])> = vectors_for_hnsw
            .iter()
            .map(|(id, v)| (*id, v.as_slice()))
            .collect();

        {
            let mut vector_storage = self.vector_storage.write();
            vector_storage
                .store_batch(&vectors_for_storage)
                .map_err(Error::Io)?;
            // Perf: Flush while lock is already held — avoids a second write() acquisition
            vector_storage.flush().map_err(Error::Io)?;
        }

        // Store payloads and update BM25 (still sequential for now)
        {
            let mut payload_storage = self.payload_storage.write();
            for point in points {
                if let Some(payload) = &point.payload {
                    payload_storage
                        .store(point.id, payload)
                        .map_err(Error::Io)?;

                    // Update BM25 text index
                    let text = Self::extract_text_from_payload(payload);
                    if !text.is_empty() {
                        self.text_index.add_document(point.id, &text);
                    }
                }
            }
            // Perf: Flush while lock is already held — avoids a second write() acquisition
            payload_storage.flush().map_err(Error::Io)?;
        }

        // Perf: Parallel HNSW insertion (CPU bound - benefits from parallelism)
        let inserted = self.index.insert_batch_parallel(vectors_for_hnsw);
        self.index.set_searching_mode();

        // Update point count
        let mut config = self.config.write();
        config.point_count = self.vector_storage.read().len();
        drop(config);
        // NOTE: index.save() removed - too slow for batch operations
        // Call collection.flush() explicitly if durability is critical

        // Invalidate stats cache so the next get_stats() recomputes fresh data.
        *self.cached_stats.lock() = None;

        // Bump write generation once per batch (CACHE-01 invalidation counter).
        //
        // Intentional placement: the bump occurs AFTER all mutations (vector
        // storage write, payload storage write, HNSW insertion, config update)
        // have completed successfully. Bumping earlier would allow a concurrent
        // reader to see the new generation before the data is consistent,
        // causing it to build a fresh plan key that matches no cached entry —
        // harmless, but wasteful. Bumping here ensures cache invalidation is
        // visible only once all writes are durable.
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(inserted)
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
        let config = self.config.read();
        let is_metadata_only = config.metadata_only;
        drop(config);

        let mut payload_storage = self.payload_storage.write();

        if is_metadata_only {
            // For metadata-only collections, only delete from payload storage
            for &id in ids {
                let old_payload = payload_storage.retrieve(id).ok().flatten();
                payload_storage.delete(id).map_err(Error::Io)?;
                self.text_index.remove_document(id);
                self.update_secondary_indexes_on_delete(id, old_payload.as_ref());
            }

            // LOCK ORDER: drop payload_storage(3) before acquiring config(1).
            let point_count = payload_storage.ids().len();
            drop(payload_storage);
            // config(1) only — all higher-numbered locks released above.
            let mut config = self.config.write();
            config.point_count = point_count;
            drop(config);
        } else {
            // For vector collections, delete from all stores
            let mut vector_storage = self.vector_storage.write();
            // Acquire cache locks once outside the loop (was N×3 lock acquisitions)
            let mut sq8_cache = self.sq8_cache.write();
            let mut binary_cache = self.binary_cache.write();
            let mut pq_cache = self.pq_cache.write();

            for &id in ids {
                let old_payload = payload_storage.retrieve(id).ok().flatten();
                vector_storage.delete(id).map_err(Error::Io)?;
                payload_storage.delete(id).map_err(Error::Io)?;
                self.index.remove(id);
                sq8_cache.remove(&id);
                binary_cache.remove(&id);
                pq_cache.remove(&id);
                self.text_index.remove_document(id);
                self.update_secondary_indexes_on_delete(id, old_payload.as_ref());
            }

            // LOCK ORDER: drop vector_storage(2), payload_storage(3), caches before acquiring config(1).
            let point_count = vector_storage.len();
            drop(vector_storage);
            drop(payload_storage);
            drop(sq8_cache);
            drop(binary_cache);
            drop(pq_cache);
            // config(1) only — all higher-numbered locks released above.
            let mut config = self.config.write();
            config.point_count = point_count;
            drop(config);

            // LOCK ORDER: sparse_indexes(9) — acquired after all lower-numbered locks released.
            // WAL-before-apply: write delete intent to WAL before mutating the index.
            #[cfg(feature = "persistence")]
            {
                let indexes = self.sparse_indexes.read();
                if !indexes.is_empty() {
                    for (name, _) in indexes.iter() {
                        let wal_path =
                            crate::index::sparse::persistence::wal_path_for_name(&self.path, name);
                        for &id in ids {
                            crate::index::sparse::persistence::wal_append_delete(&wal_path, id)?;
                        }
                    }
                }
            }

            {
                let indexes = self.sparse_indexes.read();
                for idx in indexes.values() {
                    for &id in ids {
                        idx.delete(id);
                    }
                }
            }
        }

        // Invalidate stats cache so the next get_stats() recomputes fresh data.
        *self.cached_stats.lock() = None;

        // Bump write generation once per batch (CACHE-01 invalidation counter).
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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
