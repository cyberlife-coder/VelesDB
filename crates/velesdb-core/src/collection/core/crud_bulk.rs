//! Bulk CRUD operations for Collection (`upsert_bulk`).
//!
//! Extracted from `crud.rs` (Issue #425) to keep each file under 500 NLOC.
//! These methods are optimized for high-throughput import: one durability barrier per store per batch.
//! Raw import path (`upsert_bulk_from_raw`) is in `bulk_import.rs`.
//!
//! When `async_index_builder` is configured, `upsert_bulk` uses an optimized
//! V2 path: `DirectVectorWriter` places each vector in the graph's
//! `ContiguousVectors`, and `AsyncIndexBuilder` defers linking it into the
//! graph for higher throughput. Until it is linked, every vector search finds
//! the point among the builder's pending ids, scored on what storage holds
//! for it (`collection/search/pending.rs`).

use crate::collection::types::Collection;
use crate::error::Result;
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
    /// - Vector then payload I/O, each with one durability barrier per batch
    /// - Single flush at the end (not per-point)
    /// - No HNSW index save (deferred for performance)
    /// - Faster than the previous sequential approach on large batches (5000+)
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert_bulk(&self, points: &[Point]) -> Result<usize> {
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
        // bypasses the quantized backends' insert — on a RaBitQ or SQ8
        // backend that would desynchronize the positional encoding store
        // from the node ids. Quantized collections always take the standard
        // path.
        // Loaded once: the V2 path and its builder are one decision.
        let builder = self
            .streaming
            .async_index_builder
            .load_full()
            .filter(|_| !self.storage.index.is_quantized_backend());
        let count = if let Some(aib) = builder {
            self.upsert_bulk_v2_path(&aib, &vector_refs, points, &sparse_batch)?
        } else {
            self.upsert_bulk_standard_path(&vector_refs, points, &sparse_batch)?
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
    /// Places each vector in `ContiguousVectors` and maps its id there, then
    /// queues the id for the builder to link where it is: each vector is
    /// written once (#2264).
    fn upsert_bulk_v2_path(
        &self,
        aib: &crate::collection::streaming::AsyncIndexBuilder,
        vector_refs: &[(u64, &[f32])],
        points: &[Point],
        sparse_batch: &BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>>,
    ) -> Result<usize> {
        // WAL + payload write (same durability guarantees as standard path).
        let entries = Self::payload_entries(points);
        self.store_vectors_and_payloads(vector_refs, &entries)?;

        // Place each vector in the graph's ContiguousVectors, where rerank and
        // brute force see it at once, and queue its id for the builder to link
        // that slot; until then every search scores it through the builder.
        // An index with its exact-distance features off gives the writer no
        // slot to fill: the builder then places the vectors itself.
        let writer = DirectVectorWriter::new(&self.storage.index);
        let needs_flush = match writer.write_batch_direct(vector_refs)? {
            Some(_) => aib.enqueue_placed(vector_refs.iter().map(|&(id, _)| id)),
            None => aib.enqueue(points.iter().map(|p| p.id)),
        };

        if needs_flush {
            // Buffer reached merge_threshold — flush synchronously.
            aib.flush_sync(&self.storage.index, &|ids| self.index_current(ids))?;
        }

        let count = vector_refs.len();
        self.finalize_bulk_upsert(points, entries.len(), sparse_batch)?;

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
    ) -> Result<usize> {
        let entries = Self::payload_entries(points);
        self.store_vectors_and_payloads(vector_refs, &entries)?;

        let inserted = self.bulk_index_or_defer(vector_refs);
        self.finalize_bulk_upsert(points, entries.len(), sparse_batch)?;

        Ok(inserted)
    }

    /// Post-write bookkeeping shared by the V2 and standard bulk paths:
    /// refreshes the point count, applies the sparse batch, and invalidates
    /// caches.
    ///
    /// `payload_count` is how many of `points` carry a payload: a bulk point
    /// without one leaves the stored payload as it is.
    fn finalize_bulk_upsert(
        &self,
        points: &[Point],
        payload_count: usize,
        sparse_batch: &BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>>,
    ) -> Result<()> {
        // Two statements so the guards never overlap: the one-liner form
        // evaluates the RHS first, holding vector_storage (rank 2) while
        // acquiring config (rank 1) — a decreed-order inversion.
        let point_count = self.storage.vector_storage.read().len();
        self.storage.config.write().point_count = point_count;
        self.apply_sparse_batch_bulk(sparse_batch)?;
        if payload_count == points.len() {
            self.bump_generation_with_mirror_upserts(points);
        } else {
            // The mirror's incremental upsert reads a missing payload as an
            // empty row, which a point that kept its payload is not.
            self.invalidate_caches_and_bump_generation();
        }
        Ok(())
    }

    /// Writes vectors and payloads, each store paying one durability barrier
    /// for the whole batch, then brings every index derived from the payloads
    /// in line with them. Shared by `upsert_bulk` and `upsert_bulk_from_raw`.
    ///
    /// The two stores run one after the other, on this thread, and both
    /// complete even when one fails. Each holds its storage's write guard
    /// while it writes, and no job on a rayon pool may take a collection lock
    /// (`docs/CONCURRENCY_MODEL.md`, "Collection locks and rayon"): they ran
    /// as the two halves of a global-pool `rayon::join` once, the shape
    /// `upsert` reverted for deadlocking.
    /// Stored payloads are readable whatever happened to the vectors, so the
    /// secondary indexes, the histograms and BM25 follow them before a vector
    /// error is returned; returning it first left stored payloads unindexed.
    ///
    /// LOCK ORDER: vector_storage(2), released; then payload_storage(3) → label_index(7),
    /// released → graph_range_indexes(7); then, with both storage locks
    /// released, secondary_indexes(6), stats_io_mutex(12), the BM25 WAL.
    pub(super) fn store_vectors_and_payloads(
        &self,
        vector_refs: &[(u64, &[f32])],
        entries: &[(u64, &serde_json::Value)],
    ) -> Result<()> {
        let vectors_stored = self.bulk_store_vectors(vector_refs);
        let old_payloads = self.store_payload_entries(entries);

        let old_payloads = old_payloads?;
        // The raw bulk path always maintained the secondary indexes; this one
        // never did, so an equality lookup missed every bulk-upserted point.
        self.update_secondary_indexes_for_entries(entries, &old_payloads);
        // Incremental histogram maintenance (Bug #47 + Bug #49): dedup by id
        // so only the final payload counts, then atomic decrement + increment.
        self.apply_histogram_replace_for_entries(entries, &old_payloads);
        // Issue #1797: one BM25 WAL barrier for the whole batch, after the
        // payloads it describes are stored.
        self.update_text_index_for_entries(entries, &old_payloads)?;
        vectors_stored
    }

    /// The `(id, payload)` of every point that carries a payload, in order.
    ///
    /// A bulk point without a payload stores none and leaves the stored one
    /// as it is, so it changes nothing an index derives from payloads.
    fn payload_entries(points: &[Point]) -> Vec<(u64, &serde_json::Value)> {
        points
            .iter()
            .filter_map(|p| p.payload.as_ref().map(|payload| (p.id, payload)))
            .collect()
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
        let mut storage = self.storage.vector_storage.write();
        storage.store_batch(vectors)?;
        storage.flush()?;
        drop(storage);
        Ok(())
    }

    /// Stores bulk payload entries and updates the label and range indexes,
    /// returning each entry's pre-batch payload (the first occurrence of an
    /// id gets it, a repeat gets `None`, as `collect_old_payloads` does).
    ///
    /// Shared by `upsert_bulk` and `upsert_bulk_from_raw`. `entries` holds only
    /// the points that carry a payload: one without a payload stores nothing,
    /// so it keeps its payload and every index entry derived from it.
    ///
    /// Uses `LogPayloadStorage::store_batch()` for a single WAL sync instead
    /// of per-point fsync: a batch pays one sync, not one per point.
    ///
    /// One `payload_storage` write guard spans the old-payload read, the write
    /// and the index update, so a concurrent writer cannot store a payload for
    /// these ids in between. Overwritten payloads have their labels and range
    /// entries removed before the new ones are indexed (Issue #486 and its
    /// follow-up: indexing only the incoming payloads left an overwritten
    /// point listed under its old labels too).
    ///
    /// LOCK ORDER: payload_storage(3) → label_index(7), released →
    /// graph_range_indexes(7).
    pub(super) fn store_payload_entries(
        &self,
        entries: &[(u64, &serde_json::Value)],
    ) -> Result<Vec<Option<serde_json::Value>>> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let mut storage = self.storage.payload_storage.write();
        let old_payloads =
            Self::collect_old_payloads_by_id(entries.iter().map(|&(id, _)| id), &storage);
        if let Err(error) = storage.store_batch(entries) {
            // A failed batch is not all-or-nothing: the records written before
            // the error, or all of them when only the sync failed, are readable.
            let ids: Vec<u64> = entries.iter().map(|&(id, _)| id).collect();
            self.index_payloads_after_failed_write(&storage, &ids, &old_payloads);
            return Err(error.into());
        }
        self.index_payload_writes(
            entries.iter().map(|&(id, payload)| (id, Some(payload))),
            &old_payloads,
        );
        drop(storage);
        Ok(old_payloads)
    }

    /// Applies sparse batch with WAL-before-apply for bulk insert.
    // Exclusive across WAL append and in-memory apply, so compaction cannot
    // snapshot between the two and then discard the record (WAL-before-apply).
    #[expect(clippy::significant_drop_tightening)]
    fn apply_sparse_batch_bulk(
        &self,
        sparse_batch: &BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>>,
    ) -> Result<()> {
        if sparse_batch.is_empty() {
            return Ok(());
        }
        // Serialize WAL-before-apply with snapshot commit and WAL reset.
        let mut indexes = self.query.sparse_indexes.write();
        #[cfg(feature = "persistence")]
        {
            self.append_sparse_wal_entries(sparse_batch.iter().flat_map(|(name, docs)| {
                docs.iter()
                    .map(move |(point_id, sv)| (name.as_str(), *point_id, sv))
            }))?;
        }
        for (name, docs) in sparse_batch {
            let idx = indexes.entry(name.clone()).or_default();
            idx.insert_batch_chunk(docs);
        }
        Ok(())
    }
}
