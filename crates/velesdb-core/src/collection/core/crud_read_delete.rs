//! Read and delete operations for Collection.
//!
//! Extracted from `crud.rs` to keep each file under 500 NLOC.
//! - `get()` — point retrieval by ID
//! - `delete()` — point deletion (vector + metadata paths)
//! - `len()`, `is_empty()`, `all_ids()` — collection-level accessors

use super::crud_indexing::TextWalShared;
use super::size::StoreEffect;
use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::Point;
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
    /// Retrieves points by their IDs: the rows of `ids`, read as every SELECT
    /// reads them. A graph's are its stored nodes — an embedding, a payload or
    /// both, a node without an embedding read with an empty `vector`
    /// ([`get_stored_nodes`](Self::get_stored_nodes)); a vector collection's
    /// are its vectors; a metadata collection's its payloads. An id that is no
    /// row reads `None`.
    ///
    /// TTL-expired points (payload `_veles_expires_at <= now`) are returned
    /// as `None`, like deleted points: expired-but-not-yet-swept entries are
    /// invisible on every read surface. Internal maintenance paths that must
    /// see them (TTL rebuild, snapshots, PQ training) use
    /// [`get_raw`](Self::get_raw).
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        if self.row_universe() == RowUniverse::StoredNodes {
            return self.get_stored_nodes(ids);
        }
        let now_secs = now_unix_secs();
        self.get_raw(ids)
            .into_iter()
            .map(|point| point.filter(|p| !is_payload_expired(p.payload.as_ref(), now_secs)))
            .collect()
    }

    /// Retrieves points by their IDs **without** the TTL-expiry filter.
    ///
    /// Expired-but-not-yet-swept points are returned as-is. This must stay
    /// the backing read for `rebuild_ttl_from_payloads` (agent TTL cache) and
    /// memory snapshots: filtering them out would hide unswept expired points
    /// after a restart, so `auto_expire` could never reclaim their storage.
    ///
    /// # Lock order (Issue: ABBA deadlock, repro `tests/tokio_blocking_lock_contention_repro.rs`)
    ///
    /// Acquires `vector_storage` (rank 2) before `payload_storage` (rank 3),
    /// matching [`Collection::search`]. This
    /// used to be reversed (payload then vector), which formed a classic
    /// ABBA deadlock with `search`'s vector-then-payload order: under
    /// `parking_lot`'s writer-preferring `RwLock`, two readers acquiring the
    /// same pair of locks in opposite order — with a writer from
    /// `batch_store_all` queued on either lock — can cycle forever. Confirmed
    /// live via a sustained hung run (186s, flat CPU, identical stuck frames
    /// 33s apart). Any future read/write path touching both locks must
    /// acquire vector before payload.
    #[must_use]
    pub(crate) fn get_raw(&self, ids: &[u64]) -> Vec<Option<Point>> {
        if self.row_universe() == RowUniverse::Payloads {
            // Metadata-only collections never touch vector_storage, so the
            // canonical order is trivially satisfied here.
            let payload_storage = self.storage.payload_storage.read();
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
            // For vector collections, retrieve both vector and payload —
            // vector_storage acquired first, see the lock-order note above.
            let vector_storage = self.storage.vector_storage.read();
            let payload_storage = self.storage.payload_storage.read();
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
        self.delete_counted(ids).map(|_| ())
    }

    /// Deletes points by their IDs and returns how many of them were rows.
    ///
    /// An id counts when it was a row of the collection ([`RowUniverse`]):
    /// a vector collection's vector, a metadata collection's payload, a
    /// graph's stored node — the set [`all_ids`](Self::all_ids) lists — so a
    /// missing id counts zero and an id repeated in `ids` counts once. What
    /// a vector collection stores for an id that is no row there — an edge
    /// endpoint's payload (`store_node_payload`), or a payload an earlier
    /// version let `upsert_metadata` store without a vector, with its BM25
    /// document and sparse vectors — is removed with it and not counted.
    /// Every store is cleared of each id, sparse indexes included, on every
    /// kind of collection. The count is taken under the same write locks as
    /// the delete, so two concurrent deletes of one id never both count it.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete_counted(&self, ids: &[u64]) -> Result<usize> {
        // Collect old payloads for incremental histogram maintenance.
        let old_payloads = self.collect_payloads_for_histogram(ids);

        let rows = self.row_universe();
        // Taken before any node lock or storage guard: a delete waiting for
        // a BM25 snapshot holds up no read (`text_wal_shared`).
        let text_wal = self.text_wal_shared();
        let removed = if rows == RowUniverse::Payloads {
            self.delete_metadata_only(ids, &text_wal)?
        } else {
            self.delete_vector_core_stores(ids, rows, &text_wal)?
        };
        drop(text_wal);
        // Sparse postings go with every row, a metadata row's included: a
        // SPARSE_NEAR must never reach a deleted id.
        self.delete_from_sparse_indexes(ids)?;
        self.delete_from_deferred_stores(ids);

        // Decrement histogram buckets BEFORE cache invalidation.
        self.update_histograms_on_delete(&old_payloads);

        // Issue #900: deleting a node must cascade to its edges. Otherwise the
        // edge store retains dangling edges pointing at (or from) a node that
        // no longer exists, silently corrupting the graph. First-class on
        // every collection type (agent-memory relations live on vector
        // collections); collections without edges return immediately.
        self.cascade_delete_node_edges(ids)?;

        self.bump_generation_with_mirror_deletes(ids);
        self.report_delete(ids.len());
        Ok(removed)
    }

    /// Removes every edge connected to each deleted node id (issue #900).
    ///
    /// First-class on every collection type: any collection whose edge store
    /// holds edges cascades; empty stores (the common case) return after one
    /// cheap emptiness check.
    ///
    /// # Lock ordering
    ///
    /// Runs **after** the storage / cache / label-index write locks acquired
    /// in [`delete_vector_core_stores`](Self::delete_vector_core_stores) and
    /// [`delete_metadata_only`](Self::delete_metadata_only) have been released
    /// (those helpers drop their guards before returning). The edge store uses
    /// its own internal lock chain (`edge_ids` registry → `shards[*]` in
    /// ascending order, per `docs/CONCURRENCY_MODEL.md`), acquired here with no
    /// other collection lock held — so taking them respects the documented
    /// ascending lock order and cannot deadlock.
    fn cascade_delete_node_edges(&self, ids: &[u64]) -> Result<()> {
        // Cascade whenever edges exist — the graph dimension is first-class
        // on every collection type (agent-memory relations live on vector
        // collections). Empty stores (the common case) return immediately.
        if self.graph.edge_store.is_empty() {
            return Ok(());
        }
        let mut removed_any = false;
        for &id in ids {
            // Both directions: `remove_node_edges` clears outgoing AND incoming
            // edges so no dangling edge references the deleted node.
            let before = self.graph.edge_store.outgoing_degree(id)
                + self.graph.edge_store.incoming_degree(id);
            if before > 0 {
                // WAL-before-apply (crash durability): log the cascade remove
                // before mutating the store so a crash replays the tombstone.
                // The WAL lock spans append + apply (see `add_edge`).
                let _wal_guard = self.graph.edge_wal_lock.lock();
                #[cfg(feature = "persistence")]
                crate::collection::graph::edge_wal::wal_append_remove_node(
                    &crate::collection::graph::edge_wal::wal_path_for_edges(&self.storage.path),
                    id,
                )?;
                self.graph.edge_store.remove_node_edges(id);
                removed_any = true;
            }
        }
        if removed_any {
            // Bump write generation so cached query plans referencing the now
            // removed edges are invalidated on the next query (CACHE-01).
            self.generations
                .write_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }

    /// Collects current payloads for the given IDs (for histogram decrements on delete).
    fn collect_payloads_for_histogram(&self, ids: &[u64]) -> Vec<Option<serde_json::Value>> {
        let storage = self.storage.payload_storage.read();
        ids.iter()
            .map(|&id| storage.retrieve(id).ok().flatten())
            .collect()
    }

    /// Deletes metadata-only points.
    ///
    /// Durability discipline and crash contract are identical to
    /// [`delete_vector_core_stores`](Self::delete_vector_core_stores) minus
    /// the vector store: one WAL barrier for the payload tombstones, one for
    /// the BM25 removes — O(1) fsyncs per batch under the write locks, not
    /// O(N) (finding C3).
    ///
    /// LOCK ORDER: text_wal(1a, shared, held by the caller) →
    /// payload_storage(3) → as [`Self::index_payload_writes`].
    fn delete_metadata_only(&self, ids: &[u64], text_wal: &TextWalShared<'_>) -> Result<usize> {
        let rows = RowUniverse::Payloads;
        self.write_payload_store(rows, ids, StoreEffect::StoresOrRemoves, |payload_storage| {
            // Old payloads must be read before the tombstones land — they feed
            // the secondary-index, label-index and range-index removals below.
            let old_payloads: Vec<Option<serde_json::Value>> = ids
                .iter()
                .map(|&id| payload_storage.retrieve(id).ok().flatten())
                .collect();
            let removed = count_rows(RowUniverse::Payloads, ids, &old_payloads, |_| false);

            payload_storage.delete_batch(ids)?;
            // Issue #389: WAL-before-apply for BM25 removes so crash recovery
            // replays the remove — batched, one barrier for the whole batch.
            self.change_text_index_ordered(text_wal, &[], ids)?;
            self.unindex_deleted(ids, &old_payloads);
            Ok(removed)
        })
    }

    /// Removes points from vector/payload storage, HNSW index, caches, and the
    /// label and range indexes, and returns how many ids were `rows`.
    ///
    /// # Durability: one barrier per touched store per batch (finding C3)
    ///
    /// This used to call each store's per-point `delete()` inside the loop,
    /// paying ~3 fsyncs PER POINT (vector WAL + payload WAL + BM25 WAL)
    /// while holding every write lock below — a batch of N points wedged all
    /// concurrent readers/writers of the collection behind ~3N synchronous
    /// fsyncs. Now each store receives the whole batch under a single
    /// durability barrier ([`crate::storage::MmapStorage::delete_batch`],
    /// [`crate::storage::LogPayloadStorage::delete_batch`], BM25
    /// `wal_append_batch`), mirroring the bulk-upsert discipline
    /// (`store_batch` + one sync, #1797): exactly 3 fsyncs per batch, with
    /// the rest of the lock hold being in-memory/mmap work.
    ///
    /// The barriers stay under the locks because the mmap store's hole-punch
    /// is destructive and MUST follow a durable delete frame (#898) — but
    /// they are O(1) per batch, which removes the seconds-to-minutes wedge.
    ///
    /// # Crash contract
    ///
    /// A crash mid-batch may lose the (not yet synced) tail of the batch but
    /// never corrupts:
    ///
    /// * Deletes that did not reach their barrier are lost wholesale — the
    ///   points remain fully live after reopen. All three replay paths
    ///   CRC-check each frame and stop cleanly at a torn tail: vector WAL
    ///   (`storage/mmap/wal_replay.rs::replay_wal_to_index`, op=2 frames),
    ///   payload WAL (`storage/log_payload.rs::replay_wal_from` via
    ///   `WalEntry`, 0xC4 tombstones), BM25 (`bm25_persistence_wal::
    ///   wal_replay`, Remove frames on top of the snapshot).
    /// * `MmapStorage::delete_batch` syncs every delete frame BEFORE any
    ///   hole-punch, so a punched region is always shadowed by a durable
    ///   delete record — no zero-vector resurrection (#898 invariant).
    /// * A crash BETWEEN the barriers (vector → payload → BM25) can leave a
    ///   point deleted in one store but not the next. That window is not
    ///   new — pre-fix it existed per point, between its three fsyncs — and
    ///   reopen converges: `get()`/search require the (deleted) vector, HNSW
    ///   orphans are removed by the open-time reconciliation in
    ///   `collection/core/recovery.rs`, and the leftover payload/BM25
    ///   entries are exactly what a re-issued delete of the same ids cleans
    ///   up.
    ///
    /// LOCK ORDER: text_wal(1a, shared, held by the caller) →
    /// node_locks(1c) → vector_storage(2) → payload_storage(3) → caches(4),
    /// released → as [`Self::index_payload_writes`]. The size moves by what
    /// the batch removed ([`Self::write_both_stores`]).
    fn delete_vector_core_stores(
        &self,
        ids: &[u64],
        rows: RowUniverse,
        text_wal: &TextWalShared<'_>,
    ) -> Result<usize> {
        self.write_both_stores(rows, ids, |vector_storage, payload_storage| {
            let mut pq_cache = self.storage.pq_cache.write();

            // Old payloads must be read before the tombstones land — they feed
            // the secondary-index, label-index and range-index removals below.
            let old_payloads: Vec<Option<serde_json::Value>> = ids
                .iter()
                .map(|&id| payload_storage.retrieve(id).ok().flatten())
                .collect();
            let removed = count_rows(rows, ids, &old_payloads, |id| vector_storage.contains(id));

            // ONE durability barrier per store for the whole batch (see rustdoc).
            vector_storage.delete_batch(ids)?;
            payload_storage.delete_batch(ids)?;
            // Issue #389: WAL-before-apply for BM25 removes so crash recovery
            // replays the remove — batched, one barrier for the whole batch.
            self.change_text_index_ordered(text_wal, &[], ids)?;

            for &id in ids {
                self.storage.index.remove(id);
                pq_cache.remove(&id);
            }
            drop(pq_cache);
            self.unindex_deleted(ids, &old_payloads);
            Ok(removed)
        })
    }

    /// Removes deleted rows from the secondary, label and range indexes:
    /// the entries of the payload each one carried, `old_payloads` read per
    /// id before the delete ([`Self::index_payload_writes`], a delete being
    /// a write that stores no payload).
    ///
    /// Called with `payload_storage` still write-held, so no write to these
    /// ids can index a newer payload in between.
    fn unindex_deleted(&self, ids: &[u64], old_payloads: &[Option<serde_json::Value>]) {
        self.index_payload_writes(ids.iter().map(|&id| (id, None)), old_payloads);
    }

    /// Removes IDs from the deferred indexer (persistence feature).
    #[allow(unused_variables)] // `ids` unused when persistence feature is off.
    fn delete_from_deferred_stores(&self, ids: &[u64]) {
        // Lock order: deferred_indexer(11) acquired after sparse_indexes(9)
        // released.
        #[cfg(feature = "persistence")]
        if let Some(di) = self.streaming.deferred_indexer.load().as_ref() {
            for &id in ids {
                di.remove(id);
            }
        }
    }

    /// Deletes IDs from sparse indexes with WAL-before-apply.
    // The comment below is the contract: one exclusive guard from WAL append
    // through apply, or compaction can snapshot between the phases and drop
    // the delete record.
    #[expect(clippy::significant_drop_tightening)]
    fn delete_from_sparse_indexes(&self, ids: &[u64]) -> Result<()> {
        // Hold one exclusive guard from WAL append through apply so compaction
        // cannot snapshot between the two phases and then discard the record.
        let indexes = self.query.sparse_indexes.write();
        #[cfg(feature = "persistence")]
        {
            // One barrier per index name instead of one per (name, id): the
            // nested single-append loop cost N_NAMES x N_IDS fsyncs, all held
            // under the exclusive `sparse_indexes` guard above.
            for name in indexes.keys() {
                let wal_path =
                    crate::index::sparse::persistence::wal_path_for_name(&self.storage.path, name);
                crate::index::sparse::persistence::wal_append_delete_batch(&wal_path, ids)?;
            }
        }
        for idx in indexes.values() {
            for &id in ids {
                idx.delete(id);
            }
        }
        Ok(())
    }

    /// Returns the collection's size: the vectors of a vector collection,
    /// the rows of a metadata collection, the stored nodes of a graph — with
    /// an embedding, a payload or both, each counted once.
    ///
    /// This reflects the **storage count**, not the number of points
    /// currently indexed in the HNSW graph. During a batch upsert or when
    /// deferred indexing is active, `len()` may temporarily exceed the
    /// HNSW-indexed count until the deferred merge completes. A TTL-expired
    /// point the sweep has not removed yet is still counted, while
    /// [`get`](Self::get) already reads it as `None`.
    ///
    /// O(1): every write keeps the count exact (`collection/core/size.rs`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.size()
    }

    /// Returns true if the collection's size ([`len()`](Self::len)) is zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// Returns every row id of the collection, ascending ([`RowUniverse`]):
    /// a vector collection's vectors, a metadata collection's payloads, a
    /// graph's stored nodes. This is [`all_point_ids`](Self::all_point_ids),
    /// the set `scroll_batch` enumerates and [`len`](Self::len) counts —
    /// storage's, so a TTL-expired row the sweep has not removed yet is
    /// listed, while every read surface (`get`, `scroll_batch`, a search, a
    /// `SELECT`, an aggregate, an `UPDATE`) skips it: the ids a sweep
    /// enumerates.
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.all_point_ids()
    }

    /// Returns every row id of the collection, ascending
    /// ([`all_ids`](Self::all_ids)).
    ///
    /// A payload a vector collection stores without a vector is no row of
    /// it, and is not listed: an edge endpoint's payload
    /// (`store_node_payload`), or one an earlier version let
    /// `upsert_metadata` store, which this one refuses (`VELES-016`). It is
    /// unreachable through every read, and a delete of its id removes it.
    #[must_use]
    pub fn all_point_ids(&self) -> Vec<u64> {
        let mut ids = self.row_ids();
        ids.sort_unstable();
        ids
    }

    /// Every id either store holds, a row or not, ascending: what a
    /// `TRUNCATE` deletes, so it leaves nothing behind.
    ///
    /// LOCK ORDER: vector_storage(2), released, then payload_storage(3).
    pub(crate) fn stored_ids(&self) -> Vec<u64> {
        let mut ids = self.storage.vector_storage.read().ids();
        ids.extend(self.storage.payload_storage.read().ids());
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

/// Number of distinct ids in `ids` that were `rows`, judged by what each
/// stored: a payload (`old_payloads` is index-aligned with `ids`) and, per
/// `has_vector`, a vector.
fn count_rows(
    rows: RowUniverse,
    ids: &[u64],
    old_payloads: &[Option<serde_json::Value>],
    has_vector: impl Fn(u64) -> bool,
) -> usize {
    ids.iter()
        .zip(old_payloads)
        .filter(|(&id, payload)| rows.admits(has_vector(id), payload.is_some()))
        .map(|(&id, _)| id)
        .collect::<rustc_hash::FxHashSet<u64>>()
        .len()
}
