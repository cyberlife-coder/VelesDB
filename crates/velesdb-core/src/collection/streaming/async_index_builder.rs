//! Async HNSW index builder for deferred bulk indexing.
//!
//! Queues what the HNSW index has yet to index, and builds it either
//! synchronously (via `flush_sync`) or asynchronously (future Task 4
//! integration). It queues vectors, which a build inserts, and the ids of
//! vectors `upsert_bulk`'s direct writer already placed in the graph's arena,
//! which a flush links where they are (#2264). The vector buffer is
//! searchable via brute-force scan for consistency during construction; the
//! placed vectors are in the arena, where brute force already sees them.
//!
//! # Lock ordering
//!
//! Position 11 (after `delta_buffer` at 10). Neither queue's lock (`buffer`,
//! `placed`) may be held while acquiring any lock at position ≤ 10.

use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswIndex;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Configuration for the async index builder.
///
/// Legacy configurations persisted with a `sync_mode` field are
/// accepted transparently: serde ignores unknown fields by default,
/// so the value is dropped silently on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncIndexBuilderConfig {
    /// Number of buffered vectors that triggers a build.
    #[serde(default = "default_merge_threshold")]
    pub merge_threshold: usize,

    /// Reserved — parsed but not yet wired. Flushes currently connect nodes on
    /// the global rayon pool with no segment notion, queued vectors through
    /// `HnswIndex::insert_batch_parallel` and placed ids through the same
    /// batch connect; this knob changes nothing today.
    /// Wiring it belongs to the pipeline integration tracked under
    /// issue #488 Task 4 (the same one gating this whole builder).
    #[serde(default)]
    pub segment_count: Option<usize>,
}

fn default_merge_threshold() -> usize {
    10_000
}

impl Default for AsyncIndexBuilderConfig {
    fn default() -> Self {
        Self {
            merge_threshold: default_merge_threshold(),
            segment_count: None,
        }
    }
}

/// Async HNSW index builder: queues vectors, which a flush inserts via
/// [`HnswIndex::insert_batch_parallel`], and the ids of vectors already placed
/// in the graph's arena, which a flush links where they are.
///
/// Only synchronous flush is currently supported; background-thread
/// integration into the Collection pipeline is tracked under Issue #488
/// Task 4.
///
/// Lock order position: 11 (after `delta_buffer` at 10).
#[allow(dead_code)] // Pipeline integration tracked under Issue #488 Task 4.
pub struct AsyncIndexBuilder {
    /// Vectors pending indexation, from [`Self::enqueue`]: a build inserts
    /// them, which places and links each one.
    buffer: RwLock<Vec<(u64, Vec<f32>)>>,
    /// Ids whose vectors `upsert_bulk`'s direct writer has already placed in
    /// the graph's arena and mapped, from [`Self::enqueue_placed`]: a flush
    /// links them where they are, so no vector is placed twice (#2264).
    placed: Mutex<Vec<u64>>,
    /// Configuration.
    config: AsyncIndexBuilderConfig,
    /// Whether a build is currently in progress (shared with background thread).
    building: Arc<AtomicBool>,
}

#[allow(dead_code)] // Wired into Collection pipeline in Task 4
impl AsyncIndexBuilder {
    /// Creates a new async index builder with the given configuration.
    #[must_use]
    pub fn new(config: AsyncIndexBuilderConfig) -> Self {
        Self {
            buffer: RwLock::new(Vec::new()),
            placed: Mutex::new(Vec::new()),
            config,
            building: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Enqueues vectors for deferred indexation.
    ///
    /// Returns `true` if the buffer has reached `merge_threshold`,
    /// signaling the caller to trigger a build.
    pub fn enqueue(&self, vectors: Vec<(u64, Vec<f32>)>) -> bool {
        let mut buf = self.buffer.write();
        buf.extend(vectors);
        buf.len() >= self.config.merge_threshold
    }

    /// Enqueues the ids of vectors already placed in the graph's arena and
    /// mapped there by `upsert_bulk`'s direct writer, for a flush to link
    /// where they are.
    ///
    /// Returns `true` if the queued ids have reached `merge_threshold`,
    /// signaling the caller to trigger a build.
    pub(crate) fn enqueue_placed(&self, ids: impl IntoIterator<Item = u64>) -> bool {
        let mut placed = self.placed.lock();
        placed.extend(ids);
        placed.len() >= self.config.merge_threshold
    }

    /// Returns the number of vectors currently buffered.
    #[must_use]
    pub fn buffer_len(&self) -> usize {
        self.buffer.read().len()
    }

    /// Drains and returns all buffered vectors.
    pub fn drain_buffer(&self) -> Vec<(u64, Vec<f32>)> {
        let mut buf = self.buffer.write();
        std::mem::take(&mut *buf)
    }

    /// Brute-force searches the buffer for consistency during construction.
    ///
    /// Returns `(external_id, distance)` pairs sorted by the metric ordering,
    /// truncated to `k`.
    #[must_use]
    pub fn search_buffer(
        &self,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
    ) -> Vec<(u64, f32)> {
        let buf = self.buffer.read();
        if buf.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<(u64, f32)> = buf
            .iter()
            .filter(|(_, v)| v.len() == query.len())
            .map(|(id, v)| {
                let dist = metric.calculate(query, v);
                (*id, dist)
            })
            .collect();
        drop(buf);

        metric.sort_results(&mut results);
        results.truncate(k);
        results
    }

    /// Drains both queues into the HNSW index: inserts the buffered vectors
    /// via [`HnswIndex::insert_batch_parallel`], which places and links each
    /// one, and links the queued ids where the direct writer placed their
    /// vectors, placing nothing. Returns the number of nodes it indexed.
    ///
    /// Concurrent calls are serialized: the second caller returns
    /// `Ok(0)` while the first is in progress.
    ///
    /// # Errors
    ///
    /// Returns an error if the graph's arena does not hold a slot a queued id
    /// is mapped to.
    pub fn flush_sync(&self, hnsw_index: &HnswIndex) -> crate::error::Result<usize> {
        if self.building.swap(true, Ordering::AcqRel) {
            // Another build is in progress — skip
            return Ok(0);
        }

        let vectors = self.drain_buffer();
        let inserted =
            hnsw_index.insert_batch_parallel(vectors.iter().map(|(id, v)| (*id, v.as_slice())));
        let placed = std::mem::take(&mut *self.placed.lock());
        let linked = hnsw_index.link_placed(&placed);

        self.building.store(false, Ordering::Release);

        let linked = linked?;
        tracing::debug!(
            "AsyncIndexBuilder::flush_sync: inserted {inserted}/{} vectors, linked {linked}/{} placed ids",
            vectors.len(),
            placed.len()
        );
        Ok(inserted + linked)
    }

    /// Returns `true` if a build is currently in progress.
    #[must_use]
    pub fn is_building(&self) -> bool {
        self.building.load(Ordering::Acquire)
    }

    /// Triggers a background build if the buffer is non-empty.
    ///
    /// Returns immediately — the build runs in a separate thread.
    /// If a build is already in progress, this is a no-op.
    /// The background thread calls `insert_batch_parallel` on the
    /// provided `HnswIndex` and clears the `building` flag on completion.
    /// Ids queued by `upsert_bulk`'s direct writer are left for
    /// [`Self::flush_sync`].
    pub fn trigger_build_async(&self, hnsw_index: &Arc<HnswIndex>) {
        if self.building.swap(true, Ordering::AcqRel) {
            return; // Already building
        }

        let vectors = self.drain_buffer();
        if vectors.is_empty() {
            self.building.store(false, Ordering::Release);
            return;
        }

        let index = Arc::clone(hnsw_index);
        let flag = Arc::clone(&self.building);
        let count = vectors.len();

        std::thread::spawn(move || {
            let pairs: Vec<(u64, &[f32])> =
                vectors.iter().map(|(id, v)| (*id, v.as_slice())).collect();
            let _ = index.insert_batch_parallel(pairs);
            flag.store(false, Ordering::Release);
            tracing::debug!("AsyncIndexBuilder: background build complete ({count} vectors)");
        });
    }

    /// Returns the merge threshold from the configuration.
    #[must_use]
    pub fn merge_threshold(&self) -> usize {
        self.config.merge_threshold
    }

    /// Directly sets the `building` flag for deterministic unit testing.
    ///
    /// Allows tests to inject the "already building" state without spawning a
    /// real background thread, eliminating the race condition inherent in
    /// timing-based probes.
    #[cfg(test)]
    pub(crate) fn force_set_building(&self, val: bool) {
        self.building.store(val, Ordering::SeqCst);
    }
}
