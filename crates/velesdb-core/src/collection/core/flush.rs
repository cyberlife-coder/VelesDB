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
        self.flush_core_storage()?;
        self.flush_derived_indexes()?;
        // Write the deferred vectors.idx AFTER all other flush steps.
        self.vector_storage.read().flush_index()?;
        Ok(())
    }

    /// Flushes config + vector/payload storage + drains + HNSW save.
    ///
    /// Extracted from `flush_full` to keep its CC under the Codacy limit
    /// after adding the BM25 snapshot/WAL step (#389).
    fn flush_core_storage(&self) -> Result<()> {
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
        Ok(())
    }

    /// Persists all derived indexes (secondary, sparse, BM25) in order.
    ///
    /// BM25 (issue #389) is ordered AFTER sparse so the collection-level
    /// flush semantics remain "durable → truncate WAL" uniformly for both.
    fn flush_derived_indexes(&self) -> Result<()> {
        self.flush_secondary_indexes()?;
        self.flush_sparse_indexes()?;
        self.flush_bm25_index()?;
        Ok(())
    }

    /// Persists the BM25 index as an atomic snapshot and truncates its
    /// WAL on success (issue #389).
    ///
    /// Skipped entirely when the index is empty: no snapshot file is
    /// created, so a pre-existing (non-BM25) collection reopened by
    /// newer code does not gain a spurious empty snapshot.
    fn flush_bm25_index(&self) -> Result<()> {
        if self.text_index.is_empty() {
            return Ok(());
        }
        crate::index::bm25_persistence::save_snapshot(&self.path, &self.text_index)?;
        let wal_path = crate::index::bm25_persistence_wal::wal_path_for_bm25(&self.path);
        crate::index::bm25_persistence_wal::wal_truncate(&wal_path)?;
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
    ///
    /// Under the `test-fault-injection` cargo feature, checks a
    /// process-wide flag first and returns a synthetic `Error::Io`
    /// without touching the file system. This seam lets downstream
    /// crates (velesdb-server tests in particular) exercise the
    /// rollback path of `apply_advanced_config` without needing a
    /// real disk-full or permission error. The check is a single
    /// atomic load and is optimised out entirely when the feature
    /// is disabled.
    pub(crate) fn save_config(&self) -> Result<()> {
        use std::io::Write;

        #[cfg(feature = "test-fault-injection")]
        {
            if crate::fault_injection::should_fail_save_config() {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "fault-injected save_config failure (test-fault-injection feature)",
                )));
            }
        }

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

    /// Vacuums the HNSW index of this collection, rebuilding the
    /// graph from the current vector storage and reclaiming memory
    /// occupied by tombstoned entries. Returns the number of entries
    /// compacted.
    ///
    /// This is the collection-level wrapper around
    /// [`HnswIndex::vacuum`] used by the server admin endpoint
    /// `POST /collections/{name}/index/rebuild` (finding F-21).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HNSW vacuum fails (for
    /// instance, when vector storage is disabled on the index).
    pub(crate) fn vacuum_hnsw_index(&self) -> Result<usize> {
        self.index
            .vacuum()
            .map_err(|e| Error::Index(format!("HNSW vacuum failed: {e}")))
    }

    /// Compacts the vector storage, rewriting active vectors into a
    /// contiguous layout and reclaiming disk space from deleted entries.
    ///
    /// Returns the number of bytes reclaimed.
    ///
    /// # Errors
    ///
    /// Returns an error if the compaction I/O fails.
    pub(crate) fn compact_vector_storage(&self) -> Result<usize> {
        self.vector_storage
            .write()
            .compact()
            .map_err(|e| Error::Storage(format!("storage compaction failed: {e}")))
    }

    /// Applies post-creation overrides to the advanced configuration
    /// fields and persists the updated `config.json` atomically.
    ///
    /// This is used by the server `POST /collections` handler to wire
    /// `pq_rescore_oversampling`, `deferred_indexing`, and
    /// `async_index_builder` from the REST payload after the collection
    /// has been created with its base options (HNSW, storage mode).
    /// Passing `None` leaves the corresponding field unchanged — callers
    /// that need to clear a field should pass `Some(None)` via the
    /// nested `Option`.
    ///
    /// The `Option<Option<T>>` pattern encodes three states:
    /// `None` (leave unchanged), `Some(None)` (clear the field), and
    /// `Some(Some(v))` (set the field to `v`). A clippy allow is
    /// applied locally because that is exactly what we need here.
    ///
    /// # Errors
    ///
    /// Returns an error if the updated config cannot be written to disk.
    #[allow(clippy::option_option)]
    pub(crate) fn apply_advanced_config(
        &self,
        pq_rescore_oversampling: Option<Option<u32>>,
        #[cfg(feature = "persistence")] deferred_indexing: Option<
            Option<crate::collection::streaming::DeferredIndexerConfig>,
        >,
        async_index_builder: Option<Option<crate::collection::streaming::AsyncIndexBuilderConfig>>,
    ) -> Result<()> {
        {
            let mut config = self.config.write();
            if let Some(rescore) = pq_rescore_oversampling {
                config.pq_rescore_oversampling = rescore;
            }
            #[cfg(feature = "persistence")]
            if let Some(deferred) = deferred_indexing {
                config.deferred_indexing = deferred;
            }
            if let Some(aib) = async_index_builder {
                config.async_index_builder = aib;
            }
        }
        self.save_config()
    }
}
