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

use super::rows::{split_graph_rows, AbsentFields};
use super::size::{RowStores, StoreEffect};
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::index::hnsw::direct_writer::DirectVectorWriter;
use crate::point::Point;
use crate::storage::VectorStorage;

use std::collections::BTreeMap;

/// The payload entries a bulk write stored, and each one's pre-batch
/// payload, in order ([`Collection::store_payload_entries`]).
type StoredEntries<'e> = (
    Vec<(u64, &'e serde_json::Value)>,
    Vec<Option<serde_json::Value>>,
);

/// The `(id, payload stored)` writes of the stored bulk `entries`, as the
/// payload effects take them: a bulk point without a payload is no entry —
/// it keeps its payload, its histogram values and its BM25 document.
fn stored_writes<'e>(
    entries: &[(u64, &'e serde_json::Value)],
) -> Vec<(u64, Option<&'e serde_json::Value>)> {
    entries
        .iter()
        .map(|&(id, payload)| (id, Some(payload)))
        .collect()
}

impl Collection {
    /// Bulk insert optimized for high-throughput import.
    ///
    /// # Performance
    ///
    /// This method is optimized for bulk loading:
    /// - Uses parallel HNSW insertion (rayon)
    /// - Payload then vector I/O, each with one durability barrier per batch
    /// - Single flush at the end (not per-point)
    /// - No HNSW index save (deferred for performance)
    /// - Faster than the previous sequential approach on large batches (5000+)
    ///
    /// Each kind of collection writes its rows by the rule [`Self::upsert`]
    /// writes them by ([`RowUniverse::of`]): a graph by the graph rule
    /// (`rows.rs`), a row without a vector through the node-payload write,
    /// never into the vector store; a metadata collection takes a point with
    /// an empty vector as a payload row by the same rule and refuses a
    /// vector (`VELES-014`). On every kind of collection a bulk point without
    /// a payload leaves the stored one as it is; on a graph or a metadata
    /// collection, where its id stores no row, it creates one with an empty
    /// payload.
    ///
    /// # Errors
    ///
    /// Returns an error if any point fails the checks of the collection's
    /// kind: a mismatched dimension, a vector where none is stored, a graph's
    /// checks.
    pub fn upsert_bulk(&self, points: &[Point]) -> Result<usize> {
        if points.is_empty() {
            return Ok(0);
        }
        // Parity item E + the checks of each kind run at the cold boundary,
        // before any storage lock / WAL write (shared with `upsert`).
        match self.row_universe() {
            // The graph rule folds a repeated id into one row it owns
            // (`split_graph_rows`), so it takes the batch by value: this
            // borrowed one is copied, once. `upsert_bulk_from_raw` builds
            // its rows owned and hands them over without a copy.
            RowUniverse::StoredNodes => self.upsert_bulk_graph_rows(points.to_vec()),
            RowUniverse::Payloads => self.upsert_bulk_payload_rows(points),
            RowUniverse::Vectors => {
                self.validate_vector_upsert_batch(points)?;
                let count = self.write_bulk_vector_rows(points)?;
                self.report_upsert(count);
                Ok(count)
            }
        }
    }

    /// [`Self::upsert_bulk`] on a metadata collection: the checks `upsert`
    /// runs on its rows ([`Self::validate_payload_rows`],
    /// [`Self::validate_payload_row_contents`]), then the rows written by the
    /// rule `upsert` writes them by ([`Self::write_payload_rows`]) under the
    /// bulk contract ([`AbsentFields::Keep`]): a row without a payload keeps
    /// the stored one, or creates an empty one where its id stores no row,
    /// and its sparse vectors are added to the stored ones. A repeated id is
    /// folded first, as on a graph. Returns how many rows wrote something,
    /// the count the observer hears.
    fn upsert_bulk_payload_rows(&self, points: &[Point]) -> Result<usize> {
        self.validate_payload_rows(points)?;
        self.validate_payload_row_contents(points)?;
        let (embedded, rows) = split_graph_rows(points.to_vec(), AbsentFields::Keep);
        debug_assert!(embedded.is_empty(), "a metadata row stores no vector");
        let written = self.write_payload_rows(&rows, AbsentFields::Keep)?;
        self.report_upsert(written);
        Ok(written)
    }

    /// The write of [`Self::upsert_bulk`] for a checked batch whose points
    /// all store a vector; returns how many it inserted, and the caller
    /// reports the write.
    pub(super) fn write_bulk_vector_rows(&self, points: &[Point]) -> Result<usize> {
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
        let stored_payloads = self.store_vectors_and_payloads(vector_refs, &entries)?;

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

        // Buffer reached merge_threshold — flush synchronously. Its ids the
        // flush keeps stay queued, and searchable, for the next one: the
        // builder logs them, and the write still succeeds. A failed link
        // still fails it — after the bookkeeping below, which the stored
        // points need whatever the index did with them.
        let flushed = needs_flush
            .then(|| aib.flush_sync(&self.storage.index, &|ids| self.index_current(ids)));

        let count = vector_refs.len();
        self.finalize_bulk_upsert(points, stored_payloads, sparse_batch)?;

        // Track inserts for periodic HNSW save (Issue #423 Component 3).
        #[allow(clippy::cast_possible_truncation)]
        self.generations
            .inserts_since_last_hnsw_save
            .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);

        tracing::debug!(
            "upsert_bulk V2 path: inserted {count} vectors via DirectVectorWriter + AsyncIndexBuilder"
        );

        flushed.transpose()?;
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
        let stored_payloads = self.store_vectors_and_payloads(vector_refs, &entries)?;

        let inserted = self.bulk_index_or_defer(vector_refs);
        self.finalize_bulk_upsert(points, stored_payloads, sparse_batch)?;

        Ok(inserted)
    }

    /// Post-write bookkeeping shared by the V2 and standard bulk paths:
    /// applies the sparse batch and invalidates caches. The writes moved the
    /// size themselves ([`Self::write_vector_store`],
    /// [`Self::write_payload_store`]).
    ///
    /// `payload_count` is how many of `points` stored a payload: a bulk point
    /// without one leaves the stored payload as it is.
    fn finalize_bulk_upsert(
        &self,
        points: &[Point],
        payload_count: usize,
        sparse_batch: &BTreeMap<String, Vec<(u64, crate::index::sparse::SparseVector)>>,
    ) -> Result<()> {
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

    /// Writes payloads and vectors, each store paying one durability barrier
    /// for the whole batch, then brings every index derived from the payloads
    /// in line with them; returns how many payloads it stored. Shared by
    /// `upsert_bulk` and `upsert_bulk_from_raw`.
    ///
    /// The stores follow the rule of every row write ([`Self::write_rows`]):
    /// the payloads, then the vectors, then the BM25 change — on a vector
    /// collection under the node locks of the batch's ids, so a delete of one
    /// of them lands wholly before or after, and no payload is left that no
    /// row owns, for a later payload-less bulk write of the id to bring back.
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
    /// LOCK ORDER: as [`Self::write_rows`] — on a vector collection
    /// text_wal(1a) → node_locks(1c) → payload_storage(3) → label_index(7),
    /// released → graph_range_indexes(7), all but the first two released;
    /// vector_storage(2), released; the BM25 index — then, with every one of
    /// them released, secondary_indexes(6), stats_io_mutex(12).
    pub(super) fn store_vectors_and_payloads(
        &self,
        vector_refs: &[(u64, &[f32])],
        entries: &[(u64, &serde_json::Value)],
    ) -> Result<usize> {
        let ids: Vec<u64> = vector_refs.iter().map(|&(id, _)| id).collect();
        let (stored, vectors_stored, text_changed) =
            self.write_rows(self.row_universe(), &ids, |stores| -> Result<_> {
                let payloads_stored = self.store_payload_entries(stores, entries);
                #[cfg(test)]
                super::write_hooks::between_row_stores();
                let vectors_stored = Self::bulk_store_vectors(stores, vector_refs);
                let (stored, old_payloads) = payloads_stored?;
                let writes = stored_writes(&stored);
                let text_changed = self.apply_payload_text(&writes, &old_payloads, |a, r| {
                    stores.change_text_index(a, r)
                });
                Ok(((stored, old_payloads), vectors_stored, text_changed))
            })?;
        let (stored, old_payloads) = stored;
        // The raw bulk path always maintained the secondary indexes; this one
        // never did, so an equality lookup missed every bulk-upserted point.
        self.update_secondary_indexes_for_entries(&stored, &old_payloads);
        self.apply_payload_histograms(&stored_writes(&stored), &old_payloads);
        text_changed?;
        vectors_stored.map(|()| stored.len())
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
    pub(super) fn collect_sparse_batch(
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

    /// Stores vectors in bulk via batch WAL + mmap write, through `stores`,
    /// moving the collection's size by what it changed
    /// ([`RowStores::vectors`]).
    pub(super) fn bulk_store_vectors(
        stores: &RowStores<'_>,
        vectors: &[(u64, &[f32])],
    ) -> Result<()> {
        let ids: Vec<u64> = vectors.iter().map(|&(id, _)| id).collect();
        stores.vectors(&ids, |storage| {
            storage.store_batch(vectors)?;
            storage.flush()?;
            Ok(())
        })
    }

    /// Stores bulk payload entries through `stores` and updates the label
    /// and range indexes, returning the entries it stored and each one's
    /// pre-batch payload (the first occurrence of an id gets it, a repeat
    /// gets `None`, as `collect_old_payloads` does) — every entry: the row
    /// write's vectors follow under the same locks ([`Self::write_rows`]).
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
    /// The write moves the collection's size by what it changed
    /// ([`RowStores::payloads`]).
    ///
    /// LOCK ORDER: payload_storage(3) → label_index(7), released →
    /// graph_range_indexes(7).
    pub(super) fn store_payload_entries<'e>(
        &self,
        stores: &RowStores<'_>,
        entries: &[(u64, &'e serde_json::Value)],
    ) -> Result<StoredEntries<'e>> {
        if entries.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let ids: Vec<u64> = entries.iter().map(|&(id, _)| id).collect();
        stores.payloads(&ids, StoreEffect::Stores, |storage| {
            let old_payloads = self.store_entries_in(storage, entries)?;
            Ok((entries.to_vec(), old_payloads))
        })
    }

    /// Stores `entries` in `storage`, under the caller's write guard, and
    /// updates the label and range indexes; returns each entry's pre-batch
    /// payload ([`Self::store_payload_entries`]).
    fn store_entries_in(
        &self,
        storage: &mut crate::storage::LogPayloadStorage,
        entries: &[(u64, &serde_json::Value)],
    ) -> Result<Vec<Option<serde_json::Value>>> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<u64> = entries.iter().map(|&(id, _)| id).collect();
        let old_payloads = Self::collect_old_payloads_by_id(ids.iter().copied(), storage);
        if let Err(error) = storage.store_batch(entries) {
            // A failed batch is not all-or-nothing: the records written
            // before the error, or all of them when only the sync failed,
            // are readable.
            self.index_payloads_after_failed_write(storage, &ids, &old_payloads);
            return Err(error.into());
        }
        self.index_payload_writes(
            entries.iter().map(|&(id, payload)| (id, Some(payload))),
            &old_payloads,
        );
        Ok(old_payloads)
    }

    /// Applies sparse batch with WAL-before-apply for bulk insert.
    // Exclusive across WAL append and in-memory apply, so compaction cannot
    // snapshot between the two and then discard the record (WAL-before-apply).
    #[expect(clippy::significant_drop_tightening)]
    pub(super) fn apply_sparse_batch_bulk(
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
