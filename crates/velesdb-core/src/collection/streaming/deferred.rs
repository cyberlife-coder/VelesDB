//! Deferred indexer for high-throughput sequential vector inserts.
//!
//! The [`DeferredIndexer`] buffers incoming vectors in a single write buffer
//! while they await insertion into the HNSW graph; every search reads the
//! buffered ids (`DeferredIndexer::ids`) before its index search and scores
//! those points on what storage holds for them
//! (`collection/search/pending.rs`). This decouples the write path (fast,
//! O(1) per point) from the index path (slower, O(log n) per point) and
//! enables threshold-triggered merge.
//!
//! # Single-buffer with threshold-triggered merge
//!
//! The indexer holds one buffer that accepts writes. When the buffer reaches
//! `merge_threshold` (or on flush), the collection indexes what it holds from
//! storage and then retires those entries (`DeferredMerge::finish`): an
//! entry leaves the buffer only once indexed, so a search racing the merge
//! finds each point in the index or here, and an entry written again since
//! stays for the next merge.
//!
//! # Deletes
//!
//! A delete removes the point from the buffer. A search scores buffered ids
//! on what storage holds, so a point deleted from storage is skipped there
//! too, and a delete never outlives a later write of the same id.
//!
//! # Lock ordering
//!
//! `DeferredIndexer` is at position 11 in the collection lock order; its
//! buffer (a `DeltaBuffer`) ranks with it.
//! A merge is serialized by an atomic flag, not a lock, so nothing is held
//! while it takes the storage and index locks.

use super::delta::DeltaBuffer;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

// ── Constants ────────────────────────────────────────────────────────────────

/// Default number of buffered vectors before a merge is triggered.
const DEFAULT_MERGE_THRESHOLD: usize = 1024;

/// Default maximum age of buffered data before a time-based merge (ms).
const DEFAULT_MAX_BUFFER_AGE_MS: u64 = 5000;

// ── Configuration ────────────────────────────────────────────────────────────

/// Configuration for the [`DeferredIndexer`].
///
/// Controls whether deferred indexing is enabled, how many vectors to
/// buffer before triggering a merge, and the maximum age of buffered data.
///
/// # Examples
///
/// ```
/// use velesdb_core::collection::streaming::DeferredIndexerConfig;
///
/// let config = DeferredIndexerConfig::default();
/// assert!(!config.enabled);
/// assert_eq!(config.merge_threshold, 1024);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredIndexerConfig {
    /// Whether deferred indexing is enabled (default: `false`).
    #[serde(default)]
    pub enabled: bool,

    /// Number of buffered vectors that triggers a merge into HNSW.
    #[serde(default = "default_merge_threshold")]
    pub merge_threshold: usize,

    /// Maximum age (milliseconds) of the oldest buffered vector before a
    /// time-based merge is triggered.
    ///
    /// The age is checked at write time (`push`/`extend`) and by
    /// [`DeferredIndexer::should_merge`] — there is no background timer, so
    /// an expired buffer merges on the next write or explicit check, not
    /// spontaneously. `0` makes every write signal a merge.
    #[serde(default = "default_max_buffer_age_ms")]
    pub max_buffer_age_ms: u64,
}

fn default_merge_threshold() -> usize {
    DEFAULT_MERGE_THRESHOLD
}

fn default_max_buffer_age_ms() -> u64 {
    DEFAULT_MAX_BUFFER_AGE_MS
}

impl Default for DeferredIndexerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            merge_threshold: DEFAULT_MERGE_THRESHOLD,
            max_buffer_age_ms: DEFAULT_MAX_BUFFER_AGE_MS,
        }
    }
}

// ── DeferredIndexer ──────────────────────────────────────────────────────────

/// Buffers vectors for deferred HNSW insertion with brute-force searchability.
///
/// See the [module-level docs](self) for design details.
pub struct DeferredIndexer {
    /// Write buffer — accepts pushes and is drained on merge.
    buffer: Arc<DeltaBuffer>,

    /// Set while a merge runs, so only one merge indexes the buffer at once.
    merging: AtomicBool,

    /// Configuration (immutable after construction).
    config: DeferredIndexerConfig,

    /// Millisecond timestamp (relative to `epoch`) of the first push since
    /// the last drain; [`AGE_EMPTY`] when the buffer holds no epoch-tracked
    /// entries. Lock-free: the common push path is one relaxed load.
    first_push_millis: AtomicU64,

    /// Time origin for `first_push_millis`.
    epoch: Instant,
}

/// Sentinel for `first_push_millis`: no first-push timestamp recorded.
const AGE_EMPTY: u64 = u64::MAX;

impl DeferredIndexer {
    /// Creates a new `DeferredIndexer` with the given configuration.
    ///
    /// The buffer starts inactive. If `config.enabled` is `false`, all
    /// write operations are no-ops.
    #[must_use]
    pub fn new(config: DeferredIndexerConfig) -> Self {
        Self {
            buffer: Arc::new(DeltaBuffer::new()),
            merging: AtomicBool::new(false),
            config,
            first_push_millis: AtomicU64::new(AGE_EMPTY),
            epoch: Instant::now(),
        }
    }

    /// The configuration this indexer was built with.
    #[must_use]
    pub(crate) fn config(&self) -> &DeferredIndexerConfig {
        &self.config
    }

    /// Records the first-push timestamp if none is recorded yet.
    ///
    /// One relaxed load in the steady state (timestamp already set); the CAS
    /// runs only on the first push after a drain. Losing the race to another
    /// first pusher is fine — either timestamp is a valid buffer birth time.
    #[inline]
    fn note_push_time(&self) {
        if self.first_push_millis.load(Ordering::Relaxed) == AGE_EMPTY {
            #[allow(clippy::cast_possible_truncation)]
            // Reason: millis since indexer creation; u64 covers 584M years.
            let now = self.epoch.elapsed().as_millis() as u64;
            let _ = self.first_push_millis.compare_exchange(
                AGE_EMPTY,
                now,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }

    /// Returns `true` if the oldest buffered entry has exceeded
    /// `max_buffer_age_ms`. `false` when the buffer is age-empty.
    #[inline]
    fn buffer_expired(&self) -> bool {
        let first = self.first_push_millis.load(Ordering::Relaxed);
        if first == AGE_EMPTY {
            return false;
        }
        #[allow(clippy::cast_possible_truncation)]
        // Reason: same clock domain as note_push_time.
        let now = self.epoch.elapsed().as_millis() as u64;
        now.saturating_sub(first) >= self.config.max_buffer_age_ms
    }

    /// Whether deferred indexing is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Pushes a vector into the write buffer.
    ///
    /// Activates the buffer lazily on first write. Returns `true` if
    /// the buffer has reached `merge_threshold`, signaling the caller
    /// to trigger a merge.
    ///
    /// No-op if deferred indexing is disabled.
    ///
    /// # TOCTOU note
    ///
    /// The `enabled` check and `len() >= threshold` read are not atomic with
    /// the push. This is benign: a concurrent drain may reset the count
    /// between push and the threshold check, causing a missed merge signal.
    /// The next push will re-trigger. A merge never deactivates the buffer,
    /// so a push racing it is kept.
    pub fn push(&self, id: u64, vector: Vec<f32>) -> bool {
        if !self.config.enabled {
            return false;
        }
        self.ensure_buffer_active();
        self.buffer.push(id, vector);
        self.note_push_time();
        self.buffer.len() >= self.config.merge_threshold || self.buffer_expired()
    }

    /// Batch-pushes vectors into the write buffer.
    ///
    /// Returns `true` if the buffer has reached `merge_threshold`.
    /// No-op if deferred indexing is disabled.
    pub fn extend(&self, entries: impl IntoIterator<Item = (u64, Vec<f32>)>) -> bool {
        if !self.config.enabled {
            return false;
        }
        self.ensure_buffer_active();
        self.buffer.extend(entries);
        self.note_push_time();
        self.buffer.len() >= self.config.merge_threshold || self.buffer_expired()
    }

    /// Removes `id` from the buffer: the point was deleted.
    pub fn remove(&self, id: u64) {
        self.buffer.remove(id);
    }

    /// The buffered ids, each once, sorted; empty while disabled or empty.
    #[must_use]
    pub(crate) fn ids(&self) -> Vec<u64> {
        if !self.is_searchable() {
            return Vec::new();
        }
        self.buffer.ids()
    }

    /// Starts a merge: a copy of the buffered entries, held as a
    /// [`DeferredMerge`], or `None` while another merge runs.
    pub(crate) fn begin_merge(&self) -> Option<DeferredMerge<'_>> {
        if self.merging.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(DeferredMerge {
            indexer: self,
            entries: self.buffer.entries(),
        })
    }

    /// Total number of pending (not yet indexed) vectors in the buffer.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.buffer.len()
    }

    /// Returns `true` if the buffer has reached `merge_threshold`.
    #[must_use]
    pub fn should_merge(&self) -> bool {
        self.buffer.len() >= self.config.merge_threshold || self.buffer_expired()
    }

    /// Returns `true` if deferred indexing is enabled and the buffer has
    /// searchable data.
    #[must_use]
    pub fn is_searchable(&self) -> bool {
        self.config.enabled && self.buffer.is_searchable()
    }

    /// Lazily activates the buffer if it is not already active.
    ///
    /// Uses the compare-and-swap [`try_activate`](super::delta::DeltaBuffer::try_activate)
    /// instead of a `is_active()` + `activate()` pair: the atomic transition
    /// closes the TOCTOU window between the check and the store. An
    /// `AlreadyActive` result is the expected idempotent case on the hot push
    /// path (the buffer stays active across many pushes) and is ignored here.
    fn ensure_buffer_active(&self) {
        let _ = self.buffer.try_activate();
    }
}

/// A merge of the deferred indexer's buffer into the index: the entries it
/// copied when it began. It holds the indexer's `merging` flag until it is
/// dropped — finished or not, a panic included — so no merge can leave every
/// later one skipping.
pub(crate) struct DeferredMerge<'a> {
    indexer: &'a DeferredIndexer,
    entries: Vec<(u64, Vec<f32>)>,
}

impl DeferredMerge<'_> {
    /// The ids the merge copied, for the caller to index.
    pub(crate) fn ids(&self) -> Vec<u64> {
        self.entries.iter().map(|&(id, _)| id).collect()
    }

    /// Ends the merge, retiring the copied entries of the ids `settled`
    /// names: those the merge indexed from storage, found indexed already,
    /// or found deleted. An id it did not settle (its read or insert
    /// failed) stays buffered, as does an entry written again since it was
    /// copied, which holds a newer vector, for the next merge.
    ///
    /// Entries are compared by value: an id rewritten during the merge to
    /// the very vector the merge copied (A, then B, then A again, with the
    /// merge reading B in between) is retired although the index then holds
    /// B. That takes two rewrites of one id inside one merge, the last back
    /// to the copied vector; until the id's next write, a search reaches it
    /// at B while storage holds A. A per-entry write sequence would close it;
    /// it is not kept.
    pub(crate) fn finish(self, settled: &[u64]) {
        let settled: rustc_hash::FxHashSet<u64> = settled.iter().copied().collect();
        let retired: Vec<(u64, &[f32])> = self
            .entries
            .iter()
            .filter(|(id, _)| settled.contains(id))
            .map(|(id, vector)| (*id, vector.as_slice()))
            .collect();
        self.indexer.buffer.retire(&retired);
    }
}

impl Drop for DeferredMerge<'_> {
    fn drop(&mut self) {
        if self.indexer.buffer.is_empty() {
            self.indexer
                .first_push_millis
                .store(AGE_EMPTY, Ordering::Relaxed);
        }
        self.indexer.merging.store(false, Ordering::Release);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
// Reason: clippy 1.90 similar_names flags idiomatic test bindings (ids/idx).
#[allow(clippy::similar_names)]
mod tests {
    use super::*;

    /// Helper: builds an enabled config with a custom threshold.
    fn enabled_config(threshold: usize) -> DeferredIndexerConfig {
        DeferredIndexerConfig {
            enabled: true,
            merge_threshold: threshold,
            ..DeferredIndexerConfig::default()
        }
    }

    // ── Push tests ───────────────────────────────────────────────────────

    #[test]
    fn test_deferred_push_when_enabled() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0, 0.0, 0.0]);
        idx.push(2, vec![0.0, 1.0, 0.0]);
        assert_eq!(idx.pending_count(), 2);
    }

    #[test]
    fn age_zero_makes_every_write_signal_a_merge() {
        let indexer = DeferredIndexer::new(DeferredIndexerConfig {
            enabled: true,
            merge_threshold: 1_000,
            max_buffer_age_ms: 0,
        });
        // Far below the count threshold: only the age trigger can fire.
        assert!(indexer.push(1, vec![0.0; 4]));
        assert!(indexer.should_merge());
        // A merge that empties the buffer clears the age stamp; an empty
        // buffer never reports expiry on its own.
        let merge = indexer.begin_merge().expect("test: no merge running");
        assert_eq!(merge.ids(), vec![1]);
        let settled = all_of(&merge);
        merge.finish(&settled);
        assert!(!indexer.should_merge());
    }

    #[test]
    fn unexpired_buffer_below_threshold_does_not_merge() {
        let indexer = DeferredIndexer::new(DeferredIndexerConfig {
            enabled: true,
            merge_threshold: 1_000,
            // One hour: cannot expire within the test.
            max_buffer_age_ms: 3_600_000,
        });
        assert!(!indexer.push(1, vec![0.0; 4]));
        assert!(!indexer.extend(vec![(2, vec![0.0; 4])]));
        assert!(!indexer.should_merge());
    }

    #[test]
    fn test_deferred_push_returns_true_at_threshold() {
        let idx = DeferredIndexer::new(enabled_config(3));
        assert!(!idx.push(1, vec![1.0]));
        assert!(!idx.push(2, vec![2.0]));
        assert!(idx.push(3, vec![3.0]), "third push should hit threshold");
    }

    #[test]
    fn test_deferred_push_noop_when_disabled() {
        let config = DeferredIndexerConfig::default(); // enabled=false
        let idx = DeferredIndexer::new(config);
        let triggered = idx.push(1, vec![1.0, 2.0]);
        assert!(!triggered);
        assert_eq!(idx.pending_count(), 0);
    }

    #[test]
    fn test_deferred_extend_returns_true_at_threshold() {
        let idx = DeferredIndexer::new(enabled_config(3));
        let entries = vec![(1, vec![1.0]), (2, vec![2.0]), (3, vec![3.0])];
        assert!(idx.extend(entries), "batch should hit threshold");
    }

    /// The ids a merge settled, as `finish` takes them: every one it copied.
    fn all_of(merge: &DeferredMerge<'_>) -> Vec<u64> {
        merge.ids()
    }

    // ── Ids ──────────────────────────────────────────────────────────────

    #[test]
    fn ids_names_each_buffered_point_once_and_no_deleted_one() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(3, vec![1.0]);
        idx.push(1, vec![1.0]);
        idx.push(3, vec![2.0]);
        idx.push(2, vec![1.0]);
        idx.remove(2);
        assert_eq!(idx.ids(), vec![1, 3]);
    }

    #[test]
    fn a_point_deleted_then_written_again_is_buffered() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(7, vec![1.0]);
        idx.remove(7);
        idx.push(7, vec![2.0]);
        assert_eq!(idx.ids(), vec![7]);
    }

    // ── Merge tests ──────────────────────────────────────────────────────

    #[test]
    fn a_merge_retires_what_it_indexed_and_keeps_newer_writes() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        idx.push(2, vec![2.0]);
        let merge = idx.begin_merge().expect("test: no merge running");
        assert!(idx.begin_merge().is_none(), "one merge at a time");
        // Written again while the merge indexes the first copy.
        idx.push(2, vec![9.0]);
        let settled = all_of(&merge);
        merge.finish(&settled);

        assert_eq!(idx.ids(), vec![2], "the newer write of 2 stays buffered");
        assert!(idx.begin_merge().is_some(), "the merge flag is cleared");
    }

    /// An id the merge did not settle (its read or insert failed) stays
    /// buffered for the next merge.
    #[test]
    fn a_merge_keeps_what_it_did_not_settle() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        idx.push(2, vec![2.0]);
        let merge = idx.begin_merge().expect("test: no merge running");
        merge.finish(&[1]);
        assert_eq!(idx.ids(), vec![2]);
    }

    /// A merge that never finishes — dropped, as on a panic — still clears
    /// the flag, so the next merge runs.
    #[test]
    fn a_dropped_merge_lets_the_next_one_run() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _merge = idx.begin_merge().expect("test: no merge running");
            panic!("test: a merge that fails half-way");
        }));
        assert!(unwound.is_err(), "premise: the merge panicked");
        assert!(idx.begin_merge().is_some(), "the merge flag was cleared");
        assert_eq!(idx.ids(), vec![1], "nothing was retired");
    }

    #[test]
    fn the_buffer_stays_writable_after_a_merge() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        let merge = idx.begin_merge().expect("test: no merge running");
        let settled = all_of(&merge);
        merge.finish(&settled);
        idx.push(2, vec![2.0]);
        assert_eq!(idx.pending_count(), 1, "push after a merge must succeed");
        assert!(idx.is_searchable());
    }

    // ── Config serde test ────────────────────────────────────────────────

    #[test]
    fn test_deferred_config_serde() {
        let config = DeferredIndexerConfig {
            enabled: true,
            merge_threshold: 512,
            max_buffer_age_ms: 3000,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let restored: DeferredIndexerConfig = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.enabled);
        assert_eq!(restored.merge_threshold, 512);
        assert_eq!(restored.max_buffer_age_ms, 3000);
    }

    #[test]
    fn test_deferred_config_serde_defaults() {
        let json = "{}";
        let config: DeferredIndexerConfig = serde_json::from_str(json).expect("deserialize empty");
        assert!(!config.enabled);
        assert_eq!(config.merge_threshold, DEFAULT_MERGE_THRESHOLD);
        assert_eq!(config.max_buffer_age_ms, DEFAULT_MAX_BUFFER_AGE_MS);
    }

    // ── Edge cases ───────────────────────────────────────────────────────

    #[test]
    fn test_deferred_should_merge_reflects_threshold() {
        let idx = DeferredIndexer::new(enabled_config(2));
        assert!(!idx.should_merge());
        idx.push(1, vec![1.0]);
        assert!(!idx.should_merge());
        idx.push(2, vec![2.0]);
        assert!(idx.should_merge());
    }

    #[test]
    fn test_deferred_is_enabled_reflects_config() {
        let enabled = DeferredIndexer::new(enabled_config(1024));
        assert!(enabled.is_enabled());
        let disabled = DeferredIndexer::new(DeferredIndexerConfig::default());
        assert!(!disabled.is_enabled());
    }
}
