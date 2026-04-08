//! Collection flush and durability methods.
//!
//! Extracted from `lifecycle.rs` to reduce NLOC. Contains:
//! - `flush` — fast durability (WAL + mmap, deferred HNSW save)
//! - `flush_full` — full durability including HNSW save
//! - Delta/deferred buffer draining into HNSW
//! - Secondary index and sparse index persistence

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
    /// Issue #423 Component 3: Threshold for periodic HNSW save in `flush()`.
    ///
    /// When `inserts_since_last_hnsw_save` exceeds this value, `flush()`
    /// saves the HNSW graph as a safety measure to limit recovery time.
    const HNSW_SAVE_THRESHOLD: u64 = 10_000;

    /// Fast durability flush — persists WAL + mmap but defers HNSW save.
    ///
    /// Issue #423 Component 3: `index.save()` is skipped unless the insert
    /// counter exceeds [`Self::HNSW_SAVE_THRESHOLD`]. Gap recovery on
    /// `Collection::open()` handles missing/stale HNSW data.
    ///
    /// Use [`flush_full()`](Self::flush_full) for shutdown or compaction.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn flush(&self) -> Result<()> {
        self.save_config()?;
        // Issue #423: vector_storage.flush() is now a fast path (WAL + mmap
        // only, no vectors.idx serialization). The WAL provides crash recovery
        // even with a stale index file.
        self.vector_storage.write().flush()?;
        self.payload_storage.write().flush()?;
        // Drain delta buffer into HNSW before persisting the index.
        // Lock order: delta_buffer(10) is acquired after vector_storage(2)
        // and payload_storage(3) — both already released above.
        self.drain_delta_into_index();
        // Drain deferred indexer into HNSW (position 11, after delta at 10).
        self.drain_deferred_into_index();
        // Drain async index builder buffer (V2 bulk insert path) into HNSW.
        // Without this, sub-threshold batches from upsert_bulk would remain
        // invisible to search until the buffer reaches merge_threshold.
        self.drain_async_index_builder()?;
        // Issue #423 Component 3: Save HNSW only when insert threshold
        // exceeded. Otherwise defer to flush_full() (shutdown/compaction).
        self.save_hnsw_if_threshold_exceeded()?;
        self.flush_secondary_indexes()?;
        self.flush_sparse_indexes()
    }

    /// Full durability flush including HNSW save and `vectors.idx`.
    ///
    /// Issue #423: This is equivalent to the pre-#423 `flush()` behavior.
    /// Use on graceful shutdown or before compaction to ensure the HNSW
    /// graph and vector index file are up-to-date, avoiding gap recovery
    /// and WAL replay on the next startup.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn flush_full(&self) -> Result<()> {
        self.save_config()?;
        self.vector_storage.write().flush()?;
        self.payload_storage.write().flush()?;
        self.drain_delta_into_index();
        self.drain_deferred_into_index();
        self.drain_async_index_builder()?;
        // Always save HNSW on full flush and reset the counter.
        self.index.save(&self.path)?;
        self.inserts_since_last_hnsw_save
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.flush_secondary_indexes()?;
        self.flush_sparse_indexes()?;
        // Write the deferred vectors.idx after all other flush steps.
        self.vector_storage.read().flush_index()?;
        Ok(())
    }

    /// Saves HNSW to disk only when the insert counter exceeds the threshold.
    ///
    /// Issue #423 Component 3: periodic safety save to limit crash recovery
    /// time for high-throughput workloads.
    fn save_hnsw_if_threshold_exceeded(&self) -> Result<()> {
        let count = self
            .inserts_since_last_hnsw_save
            .load(std::sync::atomic::Ordering::Relaxed);
        if count > Self::HNSW_SAVE_THRESHOLD {
            self.index.save(&self.path)?;
            self.inserts_since_last_hnsw_save
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }

    /// Drains the delta buffer into the HNSW index (if active).
    ///
    /// No-op when the delta buffer is inactive (no rebuild in progress).
    /// After draining, the buffer is empty and inactive.
    ///
    /// Filters out IDs that have been deleted from vector storage since they
    /// were buffered, preventing ghost vectors from being re-inserted into
    /// HNSW after a concurrent delete.
    ///
    /// Uses `insert_batch_parallel` for consistent batch insert performance
    /// (same strategy as `merge_deferred_batch` in crud.rs).
    ///
    /// # Lock ordering
    ///
    /// Acquires `vector_storage` (position 2) briefly for the validity
    /// check, releases it, then inserts into the index (no lock).
    /// `delta_buffer` (position 10) is acquired first via `deactivate_and_drain`.
    /// The caller must NOT hold any lower-numbered lock when calling this method.
    #[cfg(feature = "persistence")]
    fn drain_delta_into_index(&self) {
        let drained = self.delta_buffer.deactivate_and_drain();
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
        if !valid.is_empty() {
            self.index.insert_batch_parallel(valid);
        }
    }

    /// No-op stub when persistence is disabled.
    #[cfg(not(feature = "persistence"))]
    fn drain_delta_into_index(&self) {}

    /// Drains the deferred indexer into the HNSW index (if configured).
    ///
    /// No-op when deferred indexing is not configured or disabled.
    /// After draining, both buffers are empty and inactive.
    ///
    /// Filters out IDs that have been deleted from vector storage since they
    /// were buffered, preventing ghost vectors from being re-inserted into
    /// HNSW after a concurrent delete.
    ///
    /// Uses `insert_batch_parallel` for consistent batch insert performance
    /// (same strategy as `merge_deferred_batch` in crud.rs).
    ///
    /// # Lock ordering
    ///
    /// Acquires `vector_storage` (position 2) briefly for the validity
    /// check, releases it, then inserts into the index (no lock).
    /// `deferred_indexer` (position 11) is acquired first via `drain_all`.
    /// The caller must NOT hold any lower-numbered lock.
    #[cfg(feature = "persistence")]
    fn drain_deferred_into_index(&self) {
        if let Some(ref di) = self.deferred_indexer {
            let drained = di.drain_all();
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
            if !valid.is_empty() {
                self.index.insert_batch_parallel(valid);
            }
        }
    }

    /// No-op stub when persistence is disabled.
    #[cfg(not(feature = "persistence"))]
    fn drain_deferred_into_index(&self) {}

    /// Drains the async index builder buffer into the HNSW index.
    ///
    /// Ensures sub-threshold batches from the V2 `upsert_bulk` path are
    /// indexed into HNSW, making them visible to search. Without this,
    /// vectors written via `DirectVectorWriter` but not yet flushed by
    /// `AsyncIndexBuilder` would be stored but invisible to ANN search.
    ///
    /// No-op when the async index builder is not configured.
    ///
    /// # Errors
    ///
    /// Returns an error if the async index builder flush fails (e.g. lock
    /// contention or internal batch-insert error). The error is logged at
    /// `warn` level before propagation so that operational dashboards can
    /// alert on repeated failures.
    fn drain_async_index_builder(&self) -> Result<()> {
        if let Some(ref aib) = self.async_index_builder {
            match aib.flush_sync(&self.index) {
                Ok(count) if count > 0 => {
                    tracing::debug!("flush: drained {count} vectors from async index builder");
                }
                Err(e) => {
                    tracing::warn!("flush: async index builder drain failed: {e}");
                    return Err(e);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Persists property index, range index, and edge store (EPIC-009 US-005).
    fn flush_secondary_indexes(&self) -> Result<()> {
        let property_index_path = self.path.join("property_index.bin");
        self.property_index
            .read()
            .save_to_file(&property_index_path)?;

        let range_index_path = self.path.join("range_index.bin");
        self.range_index.read().save_to_file(&range_index_path)?;

        // Save EdgeStore for graph collections (BUG-1: was never persisted)
        if self.config.read().graph_schema.is_some() {
            let edge_store_path = self.path.join("edge_store.bin");
            self.edge_store.save_to_file(&edge_store_path)?;
            // Rebuild CSR read snapshot after flush so that subsequent reads
            // benefit from zero-copy neighbor lookups (EPIC-020 US-004).
            self.edge_store.build_read_snapshot();
        }

        Ok(())
    }

    /// Compacts all named sparse indexes to disk (EPIC-062 / SPARSE-04).
    fn flush_sparse_indexes(&self) -> Result<()> {
        let indexes = self.sparse_indexes.read();
        for (name, idx) in indexes.iter() {
            crate::index::sparse::persistence::compact_named(&self.path, name, idx)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Config persistence (extracted from lifecycle.rs)
// ---------------------------------------------------------------------------
impl Collection {
    /// Saves the collection configuration to disk.
    ///
    /// Uses atomic write-tmp-fsync-rename to prevent torn writes on crash.
    pub(crate) fn save_config(&self) -> Result<()> {
        use std::io::Write;

        let config = self.config.read();
        let config_path = self.path.join("config.json");
        let tmp_path = self.path.join("config.json.tmp");
        let config_data = serde_json::to_string_pretty(&*config)
            .map_err(|e| Error::Serialization(e.to_string()))?;

        let file = std::fs::File::create(&tmp_path)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(config_data.as_bytes())?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        std::fs::rename(&tmp_path, &config_path)?;
        Ok(())
    }
}
