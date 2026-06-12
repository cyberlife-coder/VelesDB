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
    #[must_use]
    pub(crate) fn get_raw(&self, ids: &[u64]) -> Vec<Option<Point>> {
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
        // Collect old payloads for incremental histogram maintenance.
        let old_payloads = self.collect_payloads_for_histogram(ids);

        if self.config.read().metadata_only {
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
        if self.edge_store.is_empty() {
            return Ok(());
        }
        let mut removed_any = false;
        for &id in ids {
            // Both directions: `remove_node_edges` clears outgoing AND incoming
            // edges so no dangling edge references the deleted node.
            let before = self.edge_store.outgoing_degree(id) + self.edge_store.incoming_degree(id);
            if before > 0 {
                // WAL-before-apply (crash durability): log the cascade remove
                // before mutating the store so a crash replays the tombstone.
                // The WAL lock spans append + apply (see `add_edge`).
                let _wal_guard = self.edge_wal_lock.lock();
                #[cfg(feature = "persistence")]
                crate::collection::graph::edge_wal::wal_append_remove_node(
                    &crate::collection::graph::edge_wal::wal_path_for_edges(&self.path),
                    id,
                )?;
                self.edge_store.remove_node_edges(id);
                removed_any = true;
            }
        }
        if removed_any {
            // Bump write generation so cached query plans referencing the now
            // removed edges are invalidated on the next query (CACHE-01).
            self.write_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }

    /// Collects current payloads for the given IDs (for histogram decrements on delete).
    fn collect_payloads_for_histogram(&self, ids: &[u64]) -> Vec<Option<serde_json::Value>> {
        let storage = self.payload_storage.read();
        ids.iter()
            .map(|&id| storage.retrieve(id).ok().flatten())
            .collect()
    }

    /// Deletes metadata-only points.
    fn delete_metadata_only(&self, ids: &[u64]) -> Result<()> {
        // LOCK ORDER: payload_storage(3) → label_index(7).
        let mut payload_storage = self.payload_storage.write();
        let mut label_idx = self.label_index.write();
        for &id in ids {
            let old_payload = payload_storage.retrieve(id).ok().flatten();
            payload_storage.delete(id)?;
            // Issue #389: WAL-before-apply for BM25 removes so crash
            // recovery replays the remove.
            #[cfg(feature = "persistence")]
            self.append_bm25_wal_remove(id)?;
            self.text_index.remove_document(id);
            self.update_secondary_indexes_on_delete(id, old_payload.as_ref());
            if let Some(ref old) = old_payload {
                label_idx.remove_from_payload(id, old);
            }
        }
        let point_count = payload_storage.ids().len();
        drop(label_idx);
        drop(payload_storage);
        self.config.write().point_count = point_count;
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
    fn delete_vector_core_stores(&self, ids: &[u64]) -> Result<()> {
        // LOCK ORDER: vector_storage(2) → payload_storage(3) → caches(4) → label_index(7).
        let mut vector_storage = self.vector_storage.write();
        let mut payload_storage = self.payload_storage.write();
        let mut sq8_cache = self.sq8_cache.write();
        let mut binary_cache = self.binary_cache.write();
        let mut pq_cache = self.pq_cache.write();
        let mut label_idx = self.label_index.write();

        for &id in ids {
            let old_payload = payload_storage.retrieve(id).ok().flatten();
            vector_storage.delete(id)?;
            payload_storage.delete(id)?;
            self.index.remove(id);
            sq8_cache.remove(&id);
            binary_cache.remove(&id);
            pq_cache.remove(&id);
            // Issue #389: WAL-before-apply for BM25 removes so crash
            // recovery replays the remove.
            #[cfg(feature = "persistence")]
            self.append_bm25_wal_remove(id)?;
            self.text_index.remove_document(id);
            self.update_secondary_indexes_on_delete(id, old_payload.as_ref());
            if let Some(ref old) = old_payload {
                label_idx.remove_from_payload(id, old);
            }
        }

        let point_count = vector_storage.len();
        drop(label_idx);
        drop(vector_storage);
        drop(payload_storage);
        drop(sq8_cache);
        drop(binary_cache);
        drop(pq_cache);
        self.config.write().point_count = point_count;
        Ok(())
    }

    /// Removes IDs from delta buffer and deferred indexer (persistence feature).
    #[allow(unused_variables)] // `ids` unused when persistence feature is off.
    fn delete_from_deferred_stores(&self, ids: &[u64]) {
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
        self.config.read().point_count
    }

    /// Returns true if the collection is empty.
    ///
    /// Uses the same cached `point_count` as [`len()`](Self::len), reflecting
    /// the storage count rather than the HNSW-indexed count.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.config.read().point_count == 0
    }

    /// Returns all point IDs in the collection.
    ///
    /// Note: Only returns IDs that have payload entries stored. Points
    /// inserted with `None` payload may not appear. For a complete set
    /// of IDs, use [`all_point_ids`](Self::all_point_ids).
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.payload_storage.read().ids()
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
        let mut ids: std::collections::BTreeSet<u64> =
            self.vector_storage.read().ids().into_iter().collect();
        for id in self.payload_storage.read().ids() {
            ids.insert(id);
        }
        ids.into_iter().collect()
    }
}
