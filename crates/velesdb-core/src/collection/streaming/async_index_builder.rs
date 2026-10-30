//! Deferred HNSW indexing for `upsert_bulk`'s V2 path.
//!
//! Queues what the HNSW index has yet to index, and indexes it in a
//! synchronous flush ([`AsyncIndexBuilder::flush_sync`]): when a queue reaches
//! `merge_threshold`, and on `flush`, compaction and reorder. It queues
//! vectors, which a flush inserts, and the ids of vectors `upsert_bulk`'s
//! direct writer already placed in the graph's arena, which a flush links
//! where they are (#2264).
//!
//! A graph walk cannot reach what is queued, so every vector search merges
//! [`AsyncIndexBuilder::search_pending`] into its results through
//! `Collection::merge_delta`, as it does the delta buffer and the deferred
//! indexer: a point `upsert_bulk` has returned for is found before any flush.
//! What a flush indexes leaves its queue only once indexed, so a search that
//! runs during the flush finds it on one side or the other.
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

/// Configuration for the async index builder.
///
/// Legacy configurations persisted with a `sync_mode` field are
/// accepted transparently: serde ignores unknown fields by default,
/// so the value is dropped silently on load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsyncIndexBuilderConfig {
    /// Number of buffered vectors that triggers a build.
    #[serde(default = "default_merge_threshold")]
    pub merge_threshold: usize,

    /// Reserved — parsed but not wired. Flushes connect nodes with no segment
    /// notion: queued vectors through `HnswIndex::insert_batch_parallel` and
    /// placed ids through `HnswIndex::link_placed`, both submitting to the one
    /// dedicated pool those two share (`graph_pool`, \#2343) rather than the
    /// global one; this knob changes nothing.
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

/// Deferred HNSW indexing: queues vectors, which a flush inserts via
/// [`HnswIndex::insert_batch_parallel`], and the ids of vectors already placed
/// in the graph's arena, which a flush links where they are. What it queues is
/// searchable through [`Self::search_pending`] until a flush has indexed it.
///
/// Lock order position: 11 (after `delta_buffer` at 10).
pub struct AsyncIndexBuilder {
    /// Vectors pending indexation, from [`Self::enqueue`]: a flush inserts
    /// them, which places and links each one.
    buffer: RwLock<Vec<(u64, Vec<f32>)>>,
    /// Ids whose vectors `upsert_bulk`'s direct writer has already placed in
    /// the graph's arena and mapped, from [`Self::enqueue_placed`]: a flush
    /// links them where they are, so no vector is placed twice (#2264).
    placed: Mutex<Vec<u64>>,
    /// Configuration.
    config: AsyncIndexBuilderConfig,
    /// Whether a flush is in progress: it makes the flush the queues' only
    /// consumer, so the prefix a flush indexed is still the queues' prefix
    /// when it removes it.
    building: AtomicBool,
}

impl AsyncIndexBuilder {
    /// Creates a new async index builder with the given configuration.
    #[must_use]
    pub fn new(config: AsyncIndexBuilderConfig) -> Self {
        Self {
            buffer: RwLock::new(Vec::new()),
            placed: Mutex::new(Vec::new()),
            config,
            building: AtomicBool::new(false),
        }
    }

    /// The configuration this builder was built with.
    #[must_use]
    pub(crate) fn config(&self) -> &AsyncIndexBuilderConfig {
        &self.config
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

    /// The number of vectors and placed ids still waiting to be indexed.
    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.buffer.read().len() + self.placed.lock().len()
    }

    /// Exact search over everything still waiting to be indexed: the top `k`
    /// as `(external_id, score)` pairs, sorted by the metric ordering.
    ///
    /// A placed id is scored from the slot `hnsw_index` maps it to — a
    /// deleted one has no mapping and is skipped, an id upserted since is
    /// scored on its current vector — in the score space of the index's
    /// exhaustive scans. A buffered vector is scored with `metric`, as the
    /// delta buffer's are.
    #[must_use]
    pub(crate) fn search_pending(
        &self,
        hnsw_index: &HnswIndex,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
    ) -> Vec<(u64, f32)> {
        // Copied out so the queue's lock (position 11) is released before the
        // index's are taken.
        let placed = self.placed.lock().clone();
        let mut pending: Vec<(u64, f32)> = if placed.is_empty() {
            Vec::new()
        } else {
            hnsw_index
                .score_ids(query, &placed, k)
                .into_iter()
                .map(Into::into)
                .collect()
        };
        let buffered = self.search_buffer(query, k, metric);
        if !buffered.is_empty() {
            pending.extend(buffered);
            metric.sort_results(&mut pending);
            pending.truncate(k);
        }
        pending
    }

    /// Brute-force search over the vector buffer: the top `k` as
    /// `(external_id, score)` pairs, sorted by the metric ordering.
    fn search_buffer(&self, query: &[f32], k: usize, metric: DistanceMetric) -> Vec<(u64, f32)> {
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

    /// Indexes both queues into the HNSW index: inserts the buffered vectors
    /// via [`HnswIndex::insert_batch_parallel`], which places and links each
    /// one, and links the queued ids where the direct writer placed their
    /// vectors, placing nothing. Returns the number of nodes it indexed.
    ///
    /// Each queue keeps what this flush indexes until it is indexed, so
    /// [`Self::search_pending`] still finds it meanwhile; a failed link
    /// therefore leaves its ids queued for the next flush.
    ///
    /// Concurrent calls are serialized: the second caller returns
    /// `Ok(0)` while the first is in progress. What it would have indexed
    /// stays queued, and searchable.
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
        let inserted = self.insert_buffered(hnsw_index);
        let linked = self.link_placed(hnsw_index);
        self.building.store(false, Ordering::Release);
        Ok(inserted + linked?)
    }

    /// Inserts a copy of the buffered vectors, then drops them from the
    /// buffer: only this flush removes from it, and `enqueue` only appends.
    fn insert_buffered(&self, hnsw_index: &HnswIndex) -> usize {
        // Copied out so the buffer's lock (position 11) is not held across
        // the index's.
        let vectors = self.buffer.read().clone();
        if vectors.is_empty() {
            return 0;
        }
        let inserted =
            hnsw_index.insert_batch_parallel(vectors.iter().map(|(id, v)| (*id, v.as_slice())));
        self.buffer.write().drain(..vectors.len());
        tracing::debug!(
            "AsyncIndexBuilder::flush_sync: inserted {inserted}/{} vectors",
            vectors.len()
        );
        inserted
    }

    /// Links the queued ids, then drops them from the queue: only this flush
    /// removes from it, and `enqueue_placed` only appends. `link_placed`
    /// links nothing when it fails, so on error the ids stay queued: their
    /// slots stay searchable through [`Self::search_pending`], and the next
    /// flush links them.
    fn link_placed(&self, hnsw_index: &HnswIndex) -> crate::error::Result<usize> {
        let placed = self.placed.lock().clone();
        if placed.is_empty() {
            return Ok(0);
        }
        let linked = hnsw_index.link_placed(&placed)?;
        self.placed.lock().drain(..placed.len());
        tracing::debug!(
            "AsyncIndexBuilder::flush_sync: linked {linked}/{} placed ids",
            placed.len()
        );
        Ok(linked)
    }

    /// Returns the merge threshold from the configuration.
    #[must_use]
    pub fn merge_threshold(&self) -> usize {
        self.config.merge_threshold
    }

    /// Directly sets the `building` flag for deterministic unit testing.
    ///
    /// Allows tests to inject the "already building" state without a
    /// concurrent flush.
    #[cfg(test)]
    pub(crate) fn force_set_building(&self, val: bool) {
        self.building.store(val, Ordering::SeqCst);
    }
}
