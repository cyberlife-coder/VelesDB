//! Async HNSW index builder for deferred bulk indexing.
//!
//! Buffers vectors and builds the HNSW index either synchronously (via
//! `flush_sync`) or asynchronously (future Task 4 integration). The buffer
//! is searchable via brute-force scan for consistency during construction.
//!
//! # Lock ordering
//!
//! Position 11 (after `delta_buffer` at 10). The internal `RwLock` on
//! `buffer` must never be held while acquiring any lock at position ≤ 10.

use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswIndex;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Configuration for the async index builder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncIndexBuilderConfig {
    /// Number of buffered vectors that triggers a build.
    #[serde(default = "default_merge_threshold")]
    pub merge_threshold: usize,

    /// Number of segments for parallel construction (default: `num_cpus`).
    #[serde(default)]
    pub segment_count: Option<usize>,

    /// Synchronous mode — `enqueue` indexes immediately instead of buffering.
    #[serde(default)]
    pub sync_mode: bool,
}

fn default_merge_threshold() -> usize {
    10_000
}

impl Default for AsyncIndexBuilderConfig {
    fn default() -> Self {
        Self {
            merge_threshold: default_merge_threshold(),
            segment_count: None,
            sync_mode: false,
        }
    }
}

/// Async HNSW index builder that buffers vectors and builds in background.
///
/// Extends the concept of `DeferredIndexer` with segmented parallel
/// construction via [`HnswSegmentBuilder`]. For the minimal implementation,
/// only synchronous flush is supported; the background thread is added in
/// Task 4 integration.
///
/// Lock order position: 11 (after `delta_buffer` at 10).
#[allow(dead_code)] // Wired into Collection pipeline in Task 4
pub struct AsyncIndexBuilder {
    /// Buffer of vectors pending indexation.
    buffer: RwLock<Vec<(u64, Vec<f32>)>>,
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

        metric.sort_results(&mut results);
        results.truncate(k);
        results
    }

    /// Drains the buffer and indexes all vectors synchronously.
    ///
    /// Uses `HnswSegmentBuilder` for segmented parallel construction
    /// when the batch is large enough.
    ///
    /// # Errors
    ///
    /// Returns an error if HNSW insertion fails.
    pub fn flush_sync(&self, hnsw_index: &HnswIndex) -> crate::error::Result<usize> {
        if self.building.swap(true, Ordering::AcqRel) {
            // Another build is in progress — skip
            return Ok(0);
        }

        let vectors = self.drain_buffer();
        let count = vectors.len();

        if count == 0 {
            self.building.store(false, Ordering::Release);
            return Ok(0);
        }

        let pairs: Vec<(u64, &[f32])> = vectors.iter().map(|(id, v)| (*id, v.as_slice())).collect();

        let inserted = hnsw_index.insert_batch_parallel(pairs);

        self.building.store(false, Ordering::Release);

        tracing::debug!("AsyncIndexBuilder::flush_sync: indexed {inserted}/{count} vectors");

        Ok(inserted)
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
}
