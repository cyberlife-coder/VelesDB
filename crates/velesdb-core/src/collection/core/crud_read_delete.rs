//! Read and delete operations for Collection.
//!
//! Extracted from `crud.rs` to keep each file under 500 NLOC.
//! - `get()` — point retrieval by ID
//! - `delete()` — point deletion (vector + metadata paths)
//! - `len()`, `is_empty()`, `all_ids()` — collection-level accessors

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::Point;
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
    /// Retrieves points by their IDs.
    ///
    /// TTL-expired points (payload `_veles_expires_at <= now`) are returned
    /// as `None`, like deleted points: expired-but-not-yet-swept entries are
    /// invisible on every read surface. Internal maintenance paths that must
    /// see them (TTL rebuild, snapshots, PQ training) use
    /// [`get_raw`](Self::get_raw).
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
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
    /// # Lock order (Issue: ABBA deadlock, see `.investigation/http-deadlock-2026-07-22/`)
    ///
    /// Acquires `vector_storage` (rank 2) before `payload_storage` (rank 3),
    /// matching [`Collection::search`](super::super::search::vector). This
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
        let config = self.storage.config.read();
        let is_metadata_only = config.metadata_only;
        drop(config);

        if is_metadata_only {
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
        // Collect old payloads for incremental histogram maintenance.
        let old_payloads = self.collect_payloads_for_histogram(ids);

        if self.storage.config.read().metadata_only {
            self.delete_metadata_only(ids)?;
        } else {
            self.delete_vector_points(ids)?;
        }

        // Decrement histogram buckets BEFORE cache invalidation.
        self.update_histograms_on_delete(&old_payloads);

        // Issue #900: deleting a node must cascade to its edges. Otherwise the
        // edge store retains dangling edges pointing at (or from) a node that
        // no longer exists, silently corrupting the graph. First-class on
        // every collection type (agent-memory relations live on vector
        // collections); collections without edges return immediately.
        self.cascade_delete_node_edges(ids)?;

        self.bump_generation_with_mirror_deletes(ids);
        Ok(())
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
    fn delete_metadata_only(&self, ids: &[u64]) -> Result<()> {
        // LOCK ORDER: payload_storage(3) → label_index(7).
        let mut payload_storage = self.storage.payload_storage.write();
        let mut label_idx = self.graph.label_index.write();

        // Old payloads must be read before the tombstones land — they feed
        // the secondary-index and label-index removals below.
        let old_payloads: Vec<Option<serde_json::Value>> = ids
            .iter()
            .map(|&id| payload_storage.retrieve(id).ok().flatten())
            .collect();

        payload_storage.delete_batch(ids)?;
        // Issue #389: WAL-before-apply for BM25 removes so crash recovery
        // replays the remove — batched, one barrier for the whole batch.
        #[cfg(feature = "persistence")]
        self.append_bm25_wal_remove_batch(ids)?;

        for (&id, old_payload) in ids.iter().zip(&old_payloads) {
            self.storage.text_index.remove_document(id);
            self.update_secondary_indexes_on_delete(id, old_payload.as_ref());
            if let Some(ref old) = old_payload {
                label_idx.remove_from_payload(id, old);
            }
        }
        let point_count = payload_storage.ids().len();
        drop(label_idx);
        drop(payload_storage);
        self.storage.config.write().point_count = point_count;
        Ok(())
    }

    /// Deletes vector points from all stores (vector, payload, index, caches, sparse, delta).
    fn delete_vector_points(&self, ids: &[u64]) -> Result<()> {
        self.delete_vector_core_stores(ids)?;
        self.delete_from_sparse_indexes(ids)?;
        self.delete_from_deferred_stores(ids);
        Ok(())
    }

    /// Removes points from vector/payload storage, HNSW index, caches, and label index.
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
    ///   orphans are removed by the 3-pass reconciliation in
    ///   `collection/core/recovery.rs`, and the leftover payload/BM25
    ///   entries are exactly what a re-issued delete of the same ids cleans
    ///   up.
    fn delete_vector_core_stores(&self, ids: &[u64]) -> Result<()> {
        // LOCK ORDER: vector_storage(2) → payload_storage(3) → caches(4) → label_index(7).
        let mut vector_storage = self.storage.vector_storage.write();
        let mut payload_storage = self.storage.payload_storage.write();
        let mut pq_cache = self.storage.pq_cache.write();
        let mut label_idx = self.graph.label_index.write();

        // Old payloads must be read before the tombstones land — they feed
        // the secondary-index and label-index removals below.
        let old_payloads: Vec<Option<serde_json::Value>> = ids
            .iter()
            .map(|&id| payload_storage.retrieve(id).ok().flatten())
            .collect();

        // ONE durability barrier per store for the whole batch (see rustdoc).
        vector_storage.delete_batch(ids)?;
        payload_storage.delete_batch(ids)?;
        // Issue #389: WAL-before-apply for BM25 removes so crash recovery
        // replays the remove — batched, applied in-memory only below.
        #[cfg(feature = "persistence")]
        self.append_bm25_wal_remove_batch(ids)?;

        for (&id, old_payload) in ids.iter().zip(&old_payloads) {
            self.storage.index.remove(id);
            pq_cache.remove(&id);
            self.storage.text_index.remove_document(id);
            self.update_secondary_indexes_on_delete(id, old_payload.as_ref());
            if let Some(ref old) = old_payload {
                label_idx.remove_from_payload(id, old);
            }
        }

        let point_count = vector_storage.len();
        drop(label_idx);
        drop(vector_storage);
        drop(payload_storage);
        drop(pq_cache);
        self.storage.config.write().point_count = point_count;
        Ok(())
    }

    /// Removes IDs from delta buffer and deferred indexer (persistence feature).
    #[allow(unused_variables)] // `ids` unused when persistence feature is off.
    fn delete_from_deferred_stores(&self, ids: &[u64]) {
        // Lock order: delta_buffer(10) acquired after sparse_indexes(9) released.
        #[cfg(feature = "persistence")]
        for &id in ids {
            self.streaming.delta_buffer.remove(id);
        }

        // Lock order: deferred_indexer(11) acquired after delta_buffer(10).
        #[cfg(feature = "persistence")]
        if let Some(ref di) = self.streaming.deferred_indexer {
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

    /// Returns the number of points stored in the collection.
    ///
    /// This reflects the **storage count** (vectors written to disk), not the
    /// number of points currently indexed in the HNSW graph. During a batch
    /// upsert or when deferred indexing is active, `len()` may temporarily
    /// exceed the HNSW-indexed count until the deferred merge completes.
    ///
    /// Perf: Uses cached `point_count` from config instead of acquiring storage lock.
    #[must_use]
    pub fn len(&self) -> usize {
        self.storage.config.read().point_count
    }

    /// Returns true if the collection is empty.
    ///
    /// Uses the same cached `point_count` as [`len()`](Self::len), reflecting
    /// the storage count rather than the HNSW-indexed count.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.storage.config.read().point_count == 0
    }

    /// Returns all point IDs in the collection.
    ///
    /// Note: Only returns IDs that have payload entries stored. Points
    /// inserted with `None` payload may not appear. For a complete set
    /// of IDs, use [`all_point_ids`](Self::all_point_ids).
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.storage.payload_storage.read().ids()
    }

    /// Returns all point IDs from both vector and payload storage.
    ///
    /// This is the authoritative set of IDs in the collection: it unions
    /// IDs from `vector_storage` (points with vectors) and
    /// `payload_storage` (points with payloads). Points inserted with
    /// `None` payload are included via the vector storage path.
    /// Returns IDs in ascending sorted order.
    /// Uses `BTreeSet` for deduplication and sorted iteration in one pass,
    /// so callers (e.g. `scroll_batch`) need not sort separately.
    #[must_use]
    pub fn all_point_ids(&self) -> Vec<u64> {
        let mut ids: std::collections::BTreeSet<u64> = self
            .storage
            .vector_storage
            .read()
            .ids()
            .into_iter()
            .collect();
        // Bound so `payload_storage` (lock-order 3) is not held across the loop.
        let payload_ids = self.storage.payload_storage.read().ids();
        for id in payload_ids {
            ids.insert(id);
        }
        ids.into_iter().collect()
    }
}
