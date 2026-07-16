//! Bulk CRUD operations for Collection (`upsert_bulk`).
//!
//! Extracted from `crud.rs` (Issue #425) to keep each file under 500 NLOC.
//! These methods are optimized for high-throughput import with parallel I/O.
//! Raw import path (`upsert_bulk_from_raw`) is in `crud_bulk_raw.rs`.
//!
//! When `async_index_builder` is configured, `upsert_bulk` uses an optimized
//! V2 path: `DirectVectorWriter` writes vectors straight into the graph's
//! `ContiguousVectors` and `AsyncIndexBuilder` defers HNSW construction for
//! higher throughput.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::index::hnsw::direct_writer::DirectVectorWriter;
use crate::point::Point;
use crate::storage::VectorStorage;

use std::collections::BTreeMap;

impl Collection {
    /// Bulk insert optimized for high-throughput import.
    ///
    /// # Performance
    ///
    /// This method is optimized for bulk loading:
    /// - Uses parallel HNSW insertion (rayon)
    /// - Parallel payload + vector I/O via `rayon::join` (Issue #424)
    /// - Single flush at the end (not per-point)
    /// - No HNSW index save (deferred for performance)
    /// - ~15x faster than previous sequential approach on large batches (5000+)
    /// - Benchmark: 25-30 Kvec/s on 768D vectors
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert_bulk(&self, points: &[Point]) -> Result<usize> {
        self.upsert_bulk_inner(points, true)
    }

    /// Bulk insert without forcing WAL fsync at the end.
    ///
    /// Identical to [`upsert_bulk`](Self::upsert_bulk) except the WAL
    /// buffer is flushed to the OS kernel (ensuring data is out of the
    /// process) but **not** fsynced to disk. This eliminates the 1-5ms
    /// per-batch fsync overhead on Windows.
    ///
    /// # Safety Contract
    ///
    /// The caller **must** call [`flush()`](Self::flush) after the final
    /// batch to establish a durability barrier. Without that final fsync,
    /// data since the last sync point may be lost on power failure.
    ///
    /// # When to Use
    ///
    /// Use this for intermediate batches in a streaming bulk import.
    /// The final batch should use [`upsert_bulk`](Self::upsert_bulk) or be
    /// followed by an explicit [`flush()`](Self::flush).
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    #[allow(dead_code)] // Reserved for future streaming ingestion surface.
    pub(crate) fn upsert_bulk_deferred_sync(&self, points: &[Point]) -> Result<usize> {
        self.upsert_bulk_inner(points, false)
    }

    /// Shared implementation for bulk insert with configurable fsync.
    fn upsert_bulk_inner(&self, points: &[Point], fsync: bool) -> Result<usize> {
        if points.is_empty() {
            return Ok(0);
        }

        // Parity item E + dimension validation at the cold boundary, before any
        // storage lock / WAL write (shared with the single-upsert path).
        let dimension = self.storage.config.read().dimension;
        self.validate_vector_upsert_batch(points, dimension)?;

        let vector_refs: Vec<(u64, &[f32])> =
            points.iter().map(|p| (p.id, p.vector.as_slice())).collect();
        let sparse_batch = Self::collect_sparse_batch(points);

        // The V2 fast path writes vectors directly into the graph store and
        // bypasses RaBitQPrecisionHnsw::insert — on a RaBitQ backend that
        // would desynchronize the positional encoding store from the node
        // ids. RaBitQ collections always take the standard path.
        let use_v2 =
            self.streaming.async_index_builder.is_some() && !self.storage.index.is_rabitq_backend();
        let count = if use_v2 {
            self.upsert_bulk_v2_path(&vector_refs, points, &sparse_batch, fsync)?
        } else {
            self.upsert_bulk_standard_path(&vector_refs, points, &sparse_batch, fsync)?
        };

        // Wave 3 Commit 9 — wire `AutoReindexManager` into the bulk hot
        // path. No-op when no manager is attached; emits a `tracing::info!`
        // event when the attached manager reports divergence. Actual
        // reindex reconstruction is out of scope for runtime-only
        // attachment and is left to the external consumer.
        self.notify_auto_reindex_after_bulk();

        Ok(count)
    }

    /// V2 optimized path: `DirectVectorWriter` + `AsyncIndexBuilder`.
    ///
    /// Writes vectors directly to `ContiguousVectors`, then enqueues them
    /// for deferred HNSW construction.
    fn upsert_bulk_v2_path(
        &self,
        vector_refs: &[(u64, &[f32])],
        points: &[Point],
        sparse_batch: &BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>>,
        fsync: bool,
    ) -> Result<usize> {
        let Some(aib) = self.streaming.async_index_builder.as_ref() else {
            return Err(Error::Config(
                "bulk v2 path requires async index builder".to_string(),
            ));
        };

        // Collect pre-batch payloads before overwriting — used for histogram decrements.
        let old_payloads = {
            let storage = self.storage.payload_storage.read();
            Self::collect_old_payloads(points, &storage)
        };

        // WAL + payload write (same durability guarantees as standard path).
        self.store_vectors_and_payloads_inner(vector_refs, points, fsync)?;

        // Write directly to the graph's ContiguousVectors so vectors are
        // immediately visible to rerank/brute-force while HNSW construction
        // is deferred.
        let writer = DirectVectorWriter::new(&self.storage.index);
        writer.write_batch_direct(vector_refs)?;

        // Enqueue for deferred HNSW construction.
        let tuples: Vec<(u64, Vec<f32>)> =
            points.iter().map(|p| (p.id, p.vector.clone())).collect();

        let needs_flush = aib.enqueue(tuples);

        if needs_flush {
            // Buffer reached merge_threshold — flush synchronously.
            aib.flush_sync(&self.storage.index)?;
        }

        let count = vector_refs.len();
        self.finalize_bulk_upsert(points, &old_payloads, sparse_batch)?;

        // Track inserts for periodic HNSW save (Issue #423 Component 3).
        #[allow(clippy::cast_possible_truncation)]
        self.generations
            .inserts_since_last_hnsw_save
            .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);

        tracing::debug!(
            "upsert_bulk V2 path: inserted {count} vectors via DirectVectorWriter + AsyncIndexBuilder"
        );

        Ok(count)
    }

    /// Standard path: synchronous HNSW insertion.
    fn upsert_bulk_standard_path(
        &self,
        vector_refs: &[(u64, &[f32])],
        points: &[Point],
        sparse_batch: &BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>>,
        fsync: bool,
    ) -> Result<usize> {
        // Collect pre-batch payloads before overwriting — used for histogram decrements.
        let old_payloads = {
            let storage = self.storage.payload_storage.read();
            Self::collect_old_payloads(points, &storage)
        };

        self.store_vectors_and_payloads_inner(vector_refs, points, fsync)?;

        let inserted = self.bulk_index_or_defer(vector_refs);
        self.finalize_bulk_upsert(points, &old_payloads, sparse_batch)?;

        Ok(inserted)
    }

    /// Post-write bookkeeping shared by the V2 and standard bulk paths:
    /// refreshes the point count, applies the sparse batch, replaces payload
    /// histograms (dedup by id), and invalidates caches.
    fn finalize_bulk_upsert(
        &self,
        points: &[Point],
        old_payloads: &[Option<serde_json::Value>],
        sparse_batch: &BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>>,
    ) -> Result<()> {
        self.storage.config.write().point_count = self.storage.vector_storage.read().len();
        self.apply_sparse_batch_bulk(sparse_batch)?;
        // Incremental histogram maintenance (Bug #47 + Bug #49): dedup by id
        // so only the final payload counts, then atomic decrement + increment.
        self.apply_histogram_replace_dedup(points, old_payloads);
        self.bump_generation_with_mirror_upserts(points);
        Ok(())
    }

    /// Writes vectors and payloads with configurable fsync behavior.
    ///
    /// When `fsync` is `false`, WAL data is written and the buffer is
    /// flushed to the OS kernel, but `sync_all()` is skipped. This
    /// eliminates the 1-5ms per-batch overhead on Windows for
    /// intermediate streaming batches.
    fn store_vectors_and_payloads_inner(
        &self,
        vector_refs: &[(u64, &[f32])],
        points: &[Point],
        fsync: bool,
    ) -> Result<()> {
        #[cfg(feature = "persistence")]
        {
            let (vec_result, pay_result) = rayon::join(
                || self.bulk_store_vectors_inner(vector_refs, fsync),
                || self.bulk_store_payloads_inner(points, fsync),
            );
            vec_result?;
            pay_result?;
        }

        #[cfg(not(feature = "persistence"))]
        {
            self.bulk_store_vectors_inner(vector_refs, fsync)?;
            self.bulk_store_payloads_inner(points, fsync)?;
        }

        Ok(())
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
    pub(super) fn bulk_store_vectors(&self, vectors: &[(u64, &[f32])]) -> Result<()> {
        self.bulk_store_vectors_inner(vectors, true)
    }

    /// Stores vectors with configurable fsync behavior.
    ///
    /// When `fsync` is `false`, `store_batch()` writes WAL entries to the
    /// `BufWriter` but `flush()` is skipped entirely. The mmap write is
    /// still performed so the data is immediately readable in-process.
    fn bulk_store_vectors_inner(&self, vectors: &[(u64, &[f32])], fsync: bool) -> Result<()> {
        let mut storage = self.storage.vector_storage.write();
        storage.store_batch(vectors)?;
        if fsync {
            storage.flush()?;
        }
        Ok(())
    }

    /// Stores payloads and updates BM25 text index + label index in bulk.
    ///
    /// Uses `LogPayloadStorage::store_batch()` for a single WAL sync instead
    /// of per-point fsync, improving bulk insert throughput by 10-50x.
    ///
    /// When `fsync` is `false`, WAL entries are written and the buffer is
    /// flushed to the OS kernel, but `sync_all()` is skipped.
    fn bulk_store_payloads_inner(&self, points: &[Point], fsync: bool) -> Result<()> {
        let entries: Vec<(u64, &serde_json::Value)> = points
            .iter()
            .filter_map(|p| p.payload.as_ref().map(|pl| (p.id, pl)))
            .collect();

        if fsync {
            self.storage.payload_storage.write().store_batch(&entries)?;
        } else {
            self.storage
                .payload_storage
                .write()
                .store_batch_deferred(&entries)?;
        }

        // Issue #425: BM25 skip — when no point has a payload AND the BM25
        // index is empty, skip the text index loop entirely. The bulk path
        // inserts fresh points (no old documents to remove), so the loop
        // body would be a no-op for every point.
        if !entries.is_empty() || !self.storage.text_index.is_empty() {
            for point in points {
                self.update_text_index(point)?;
            }
        }

        // Issue #486: Update label index for bulk-inserted points.
        // The bulk path previously skipped label indexing (handled in
        // per_point_updates for the single-upsert path). Without this,
        // MATCH queries with label patterns (e.g., `(d:Doc)`) return
        // empty results for points inserted via upsert_bulk / REST API.
        Self::update_label_index_bulk(&self.graph.label_index, points);

        Ok(())
    }

    /// Batch-updates the label index for bulk-inserted points.
    ///
    /// For the bulk path, points are always new inserts (no old payload to
    /// remove from the label index), so we only need to index the new payloads.
    ///
    /// LOCK ORDER: label_index(7) — after payload_storage(3).
    fn update_label_index_bulk(
        label_index: &parking_lot::RwLock<crate::collection::graph::LabelIndex>,
        points: &[Point],
    ) {
        if !Self::any_point_has_labels(points) {
            return;
        }
        let mut label_idx = label_index.write();
        for point in points {
            if let Some(ref payload) = point.payload {
                label_idx.index_from_payload(point.id, payload);
            }
        }
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
            self.append_sparse_wal_entries(sparse_batch.iter().flat_map(|(name, docs)| {
                docs.iter()
                    .map(move |(point_id, sv)| (name.as_str(), *point_id, sv))
            }))?;
        }
        let mut indexes = self.query.sparse_indexes.write();
        for (name, docs) in sparse_batch {
            let idx = indexes.entry(name.clone()).or_default();
            idx.insert_batch_chunk(docs);
        }
        Ok(())
    }
}
