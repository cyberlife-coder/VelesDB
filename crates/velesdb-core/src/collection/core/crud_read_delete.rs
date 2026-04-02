//! Read and delete operations for Collection.
//!
//! Extracted from `crud.rs` to keep each file under 500 NLOC.
//! - `get()` — point retrieval by ID
//! - `delete()` — point deletion (vector + metadata paths)
//! - `len()`, `is_empty()`, `all_ids()` — collection-level accessors

use crate::collection::types::Collection;
use crate::error::Result;
use crate::index::VectorIndex;
use crate::point::Point;
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
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
        // LOCK ORDER: payload_storage(3) → label_index(7).
        let mut payload_storage = self.payload_storage.write();
        let mut label_idx = self.label_index.write();
        for &id in ids {
            let old_payload = payload_storage.retrieve(id).ok().flatten();
            payload_storage.delete(id)?;
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
    #[must_use]
    pub fn all_point_ids(&self) -> Vec<u64> {
        let mut ids: std::collections::HashSet<u64> =
            self.vector_storage.read().ids().into_iter().collect();
        for id in self.payload_storage.read().ids() {
            ids.insert(id);
        }
        ids.into_iter().collect()
    }
}
