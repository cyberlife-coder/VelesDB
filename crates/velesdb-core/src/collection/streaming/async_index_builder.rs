//! Deferred HNSW indexing for `upsert_bulk`'s V2 path.
//!
//! Queues what the HNSW index has yet to index, and indexes it in a
//! synchronous flush ([`AsyncIndexBuilder::flush_sync`]): when a queue reaches
//! `merge_threshold`, and on `flush`, compaction and reorder. It queues the
//! ids of stored points the index lacks, which a flush inserts from what
//! storage holds for them then, and the ids of vectors `upsert_bulk`'s direct
//! writer already placed in the graph's arena, which a flush links where they
//! are (#2264).
//!
//! A graph walk cannot reach what is queued, so every vector search reads
//! `AsyncIndexBuilder::pending_ids` before its index search, scores those
//! points itself and merges them with its results
//! (`collection/search/pending.rs`): a point `upsert_bulk` has returned for
//! is found before any flush. What a flush indexes leaves its queue only once
//! indexed, which is what lets a snapshot taken before a walk cover every
//! point the walk can miss.
//!
//! A queue can outgrow `merge_threshold`: a call that finds another flush
//! running leaves what it queued to the next one, and a flush whose link
//! fails keeps its ids queued. Every `upsert_bulk` that finds a queue at or
//! over the threshold flushes again, and returns the error of a link that
//! keeps failing, so the queue grows only while every write reports why.
//!
//! # Lock ordering
//!
//! Position 11 (after `sparse_indexes` at 9). Neither queue's lock (`buffer`,
//! `placed`) may be held while acquiring any lock at position ≤ 10.

use crate::index::hnsw::HnswIndex;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

/// Holds a builder's `building` flag for one flush and clears it when
/// dropped, so a flush that panics does not leave every later one skipping.
struct Building<'a>(&'a AtomicBool);

impl<'a> Building<'a> {
    /// Sets the flag, or `None` when another flush holds it.
    fn start(flag: &'a AtomicBool) -> Option<Self> {
        (!flag.swap(true, Ordering::AcqRel)).then_some(Self(flag))
    }
}

impl Drop for Building<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

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

/// Deferred HNSW indexing: queues the ids of stored points the index lacks,
/// which a flush inserts from what storage holds for them then (unless the
/// graph holds that already), and the ids of vectors already placed
/// in the graph's arena, which a flush links where they are. What it queues is
/// read by every search through `Self::pending_ids` until a flush has
/// indexed it.
///
/// Lock order position: 11 (after `sparse_indexes` at 9).
pub struct AsyncIndexBuilder {
    /// Ids pending insertion, from [`Self::enqueue`]: a flush inserts what
    /// storage holds for each then, which places and links it.
    buffer: Mutex<Vec<u64>>,
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
            buffer: Mutex::new(Vec::new()),
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

    /// Enqueues the ids of stored points for deferred insertion: the flush
    /// reads each one's vector from storage then, so a point overwritten or
    /// deleted in between is indexed as it is, or not at all.
    ///
    /// Returns `true` if the buffer has reached `merge_threshold`,
    /// signaling the caller to trigger a build.
    pub fn enqueue(&self, ids: impl IntoIterator<Item = u64>) -> bool {
        let mut buf = self.buffer.lock();
        buf.extend(ids);
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
        self.buffer.lock().len() + self.placed.lock().len()
    }

    /// The ids still waiting to be indexed, placed or buffered, each once,
    /// sorted. Copied out under each queue's lock in turn, so no lock is held
    /// while a search scores them.
    ///
    /// The queues keep an id written twice twice, which a flush indexes once;
    /// deduplicating here rather than at enqueue keeps a second write that
    /// lands while a flush is linking the first from being dropped unlinked.
    #[must_use]
    pub(crate) fn pending_ids(&self) -> Vec<u64> {
        let mut ids = self.placed.lock().clone();
        ids.extend(self.buffer.lock().iter().copied());
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Indexes both queues into the HNSW index: hands the buffered ids to
    /// `index_current`, which indexes what storage holds for each now and
    /// returns the ids it settled (inserted, held by the graph already as
    /// storage holds them, or deleted), and links the queued placed ids
    /// where the direct writer placed their vectors, placing nothing.
    /// Returns the number of buffered ids settled plus the nodes linked.
    ///
    /// Each queue keeps an id until it is indexed, so `Self::pending_ids`
    /// still names it meanwhile; an id `index_current` did not settle, and
    /// the ids of a failed link, stay queued for the next flush.
    ///
    /// Concurrent calls are serialized: the second caller returns
    /// `Ok(0)` while the first is in progress. What it would have indexed
    /// stays queued, and searchable.
    ///
    /// # Errors
    ///
    /// Returns an error if the graph's arena does not hold a slot a queued id
    /// is mapped to.
    pub fn flush_sync(
        &self,
        hnsw_index: &HnswIndex,
        index_current: &dyn Fn(&[u64]) -> Vec<u64>,
    ) -> crate::error::Result<usize> {
        let Some(_building) = Building::start(&self.building) else {
            // Another build is in progress — skip
            return Ok(0);
        };
        let settled = self.index_buffered(index_current);
        let linked = self.link_placed(hnsw_index)?;
        Ok(settled + linked)
    }

    /// Hands the buffered ids to `index_current`, then drops from the buffer
    /// the ones it settled; the others stay, ahead of what was queued since.
    /// Only this flush removes from the buffer, and `enqueue` only appends,
    /// so the ids copied are still the buffer's prefix.
    fn index_buffered(&self, index_current: &dyn Fn(&[u64]) -> Vec<u64>) -> usize {
        // Copied out so the buffer's lock (position 11) is not held across
        // the storage's or the index's.
        let ids = self.buffer.lock().clone();
        if ids.is_empty() {
            return 0;
        }
        let settled: rustc_hash::FxHashSet<u64> = index_current(&ids).into_iter().collect();
        let mut buffer = self.buffer.lock();
        let queued_since = buffer.split_off(ids.len());
        buffer.retain(|id| !settled.contains(id));
        let kept = buffer.len();
        buffer.extend(queued_since);
        drop(buffer);
        tracing::debug!(
            "AsyncIndexBuilder::flush_sync: settled {}/{} buffered ids",
            ids.len() - kept,
            ids.len()
        );
        ids.len() - kept
    }

    /// Links the queued ids, then drops them from the queue: only this flush
    /// removes from it, and `enqueue_placed` only appends. `link_placed`
    /// links nothing when it fails, so on error the ids stay queued: their
    /// slots stay searchable through `Self::pending_ids`, and the next
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
