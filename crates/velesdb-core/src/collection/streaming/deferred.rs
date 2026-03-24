//! Deferred indexer for high-throughput sequential vector inserts.
//!
//! The [`DeferredIndexer`] buffers incoming vectors in a single write buffer
//! and exposes them to search via brute-force scan while they await insertion
//! into the HNSW graph. This decouples the write path (fast, O(1) per point)
//! from the index path (slower, O(log n) per point) and enables
//! threshold-triggered merge.
//!
//! # Single-buffer with threshold-triggered merge
//!
//! The indexer holds one buffer that accepts writes. When the buffer reaches
//! `merge_threshold`, [`swap_and_drain`](DeferredIndexer::swap_and_drain)
//! drains it and returns the vectors for the caller to batch-insert into HNSW.
//!
//! # Deleted IDs
//!
//! When a point is deleted while buffered, its ID is recorded in a
//! `deleted_ids` set. Search results are filtered against this set so that
//! deleted vectors never surface. The set is cleared on drain (the HNSW
//! tombstone system takes over after merge).
//!
//! # Lock ordering
//!
//! `DeferredIndexer` is above `DeltaBuffer` (position 10) in the lock order.
//! The `swap_lock` (position 10.1) must never be held while acquiring any
//! lower-numbered lock.

use super::delta::DeltaBuffer;
use crate::distance::DistanceMetric;
use parking_lot::{Mutex, RwLock};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferredIndexerConfig {
    /// Whether deferred indexing is enabled (default: `false`).
    #[serde(default)]
    pub enabled: bool,

    /// Number of buffered vectors that triggers a merge into HNSW.
    #[serde(default = "default_merge_threshold")]
    pub merge_threshold: usize,

    /// Maximum age (milliseconds) of the oldest buffered vector before a
    /// time-based merge is triggered.
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

    /// Serializes swap-and-drain operations so only one drain runs at a time.
    swap_lock: Mutex<()>,

    /// IDs deleted while in the buffer. Filtered out of search results.
    /// Uses `FxHashSet` for faster integer hashing on the hot search path.
    deleted_ids: RwLock<FxHashSet<u64>>,

    /// Configuration (immutable after construction).
    config: DeferredIndexerConfig,
}

impl DeferredIndexer {
    /// Creates a new `DeferredIndexer` with the given configuration.
    ///
    /// The buffer starts inactive. If `config.enabled` is `false`, all
    /// write operations are no-ops.
    #[must_use]
    pub fn new(config: DeferredIndexerConfig) -> Self {
        Self {
            buffer: Arc::new(DeltaBuffer::new()),
            swap_lock: Mutex::new(()),
            deleted_ids: RwLock::new(FxHashSet::default()),
            config,
        }
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
    /// The next push will re-trigger.
    ///
    /// A previous TOCTOU window existed where `swap_and_drain` could
    /// deactivate the buffer between `ensure_buffer_active` and the
    /// underlying `buffer.push`, causing the vector to be silently dropped.
    /// This is fixed: `swap_and_drain` now re-activates the buffer after
    /// draining so pushes between drain and the next merge succeed.
    pub fn push(&self, id: u64, vector: Vec<f32>) -> bool {
        if !self.config.enabled {
            return false;
        }
        self.ensure_buffer_active();
        self.buffer.push(id, vector);
        self.buffer.len() >= self.config.merge_threshold
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
        self.buffer.len() >= self.config.merge_threshold
    }

    /// Marks `id` as deleted, removing it from the buffer.
    ///
    /// The ID is added to `deleted_ids` so that search results are filtered
    /// even if the vector was already snapshot for a concurrent search.
    pub fn remove(&self, id: u64) {
        self.buffer.remove(id);
        self.deleted_ids.write().insert(id);
    }

    /// Brute-force searches the buffer, filtering deleted IDs.
    ///
    /// Results are sorted by the metric ordering and truncated to `k`.
    ///
    /// To compensate for post-filter attrition, the buffer is queried with
    /// `k + deleted_ids.len()` candidates. This is bounded: `deleted_ids`
    /// never exceeds `merge_threshold` entries (cleared on every drain).
    ///
    /// # TOCTOU note
    ///
    /// The `deleted_ids` snapshot is read under a separate lock from the
    /// buffer search. A concurrent delete between the buffer snapshot and the
    /// `deleted_ids` read is benign: the ID will be filtered on the next
    /// search after the delete completes.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, metric: DistanceMetric) -> Vec<(u64, f32)> {
        let deleted = self.deleted_ids.read();
        let overfetch = k.saturating_add(deleted.len());
        let buffer_results = self.buffer.search(query, overfetch, metric);
        let mut filtered = filter_deleted(buffer_results, &deleted);
        drop(deleted);
        metric.sort_results(&mut filtered);
        filtered.truncate(k);
        filtered
    }

    /// Merges HNSW results with deferred buffer results.
    ///
    /// Buffer is authoritative on duplicate IDs (more recent data): when a
    /// point is upserted while deferred indexing is active, the new vector
    /// goes to the buffer while HNSW still holds the stale vector. On ID
    /// conflict the buffer score is kept, mirroring `merge_with_delta` in
    /// `delta.rs`.
    ///
    /// Deleted IDs are filtered from buffer results but not from HNSW
    /// results (HNSW has its own tombstone system).
    #[must_use]
    pub fn merge_with_hnsw(
        &self,
        hnsw_results: Vec<(u64, f32)>,
        query: &[f32],
        k: usize,
        metric: DistanceMetric,
    ) -> Vec<(u64, f32)> {
        let buffer_results = self.search(query, k, metric);
        if buffer_results.is_empty() {
            return hnsw_results;
        }
        // Buffer holds more-recent data (upserts route through buffer, not HNSW).
        // On ID conflict, keep the buffer score.
        let buffer_ids: FxHashSet<u64> = buffer_results.iter().map(|(id, _)| *id).collect();
        let mut combined: Vec<(u64, f32)> = hnsw_results
            .into_iter()
            .filter(|(id, _)| !buffer_ids.contains(id))
            .collect();
        combined.extend(buffer_results);
        metric.sort_results(&mut combined);
        combined.truncate(k);
        combined
    }

    /// Drains the buffer and returns vectors for HNSW insertion.
    ///
    /// After this call the buffer is empty but **re-activated** so that
    /// pushes arriving between drain and the next merge are not silently
    /// dropped. The `deleted_ids` set is cleared because the caller is
    /// expected to apply deletions to HNSW after merge.
    ///
    /// Serialized by an internal mutex so concurrent calls are safe (the
    /// second caller gets an empty drain).
    pub fn swap_and_drain(&self) -> Vec<(u64, Vec<f32>)> {
        let _guard = self.swap_lock.lock();
        let drained = self.buffer.deactivate_and_drain();
        self.deleted_ids.write().clear();
        // Re-activate the buffer so pushes between drain and next merge
        // are not silently dropped (fixes TOCTOU race with concurrent push).
        self.buffer.activate();
        drained
    }

    /// Total number of pending (not yet indexed) vectors in the buffer.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.buffer.len()
    }

    /// Returns `true` if the buffer has reached `merge_threshold`.
    #[must_use]
    pub fn should_merge(&self) -> bool {
        self.buffer.len() >= self.config.merge_threshold
    }

    /// Returns `true` if deferred indexing is enabled and the buffer has
    /// searchable data.
    #[must_use]
    pub fn is_searchable(&self) -> bool {
        self.config.enabled && self.buffer.is_searchable()
    }

    /// Drains all vectors from the buffer (for shutdown / flush).
    ///
    /// Clears `deleted_ids`. After this call the buffer is empty and
    /// inactive.
    pub fn drain_all(&self) -> Vec<(u64, Vec<f32>)> {
        let _guard = self.swap_lock.lock();
        let all = self.buffer.deactivate_and_drain();
        self.deleted_ids.write().clear();
        all
    }

    /// Lazily activates the buffer if it is not already active.
    fn ensure_buffer_active(&self) {
        if !self.buffer.is_active() {
            self.buffer.activate();
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Filters out deleted IDs from a result set.
fn filter_deleted(results: Vec<(u64, f32)>, deleted: &FxHashSet<u64>) -> Vec<(u64, f32)> {
    if deleted.is_empty() {
        return results;
    }
    results
        .into_iter()
        .filter(|(id, _)| !deleted.contains(id))
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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

    // ── Search tests ─────────────────────────────────────────────────────

    #[test]
    fn test_deferred_search_finds_buffered_vectors() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0, 0.0]);
        idx.push(2, vec![0.0, 1.0]);

        let results = idx.search(&[1.0, 0.0], 2, DistanceMetric::Cosine);
        assert_eq!(results.len(), 2);
        // Cosine: id=1 (identical to query) should be first
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_deferred_search_filters_deleted_ids() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0, 0.0, 0.0]);
        idx.push(2, vec![0.0, 1.0, 0.0]);
        idx.push(3, vec![0.0, 0.0, 1.0]);
        idx.remove(2);

        let results = idx.search(&[1.0, 0.0, 0.0], 10, DistanceMetric::Euclidean);
        let ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
        assert!(!ids.contains(&2), "deleted ID 2 must not appear in results");
        assert_eq!(ids.len(), 2);
    }

    // ── Swap and drain tests ─────────────────────────────────────────────

    #[test]
    fn test_deferred_swap_and_drain() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        idx.push(2, vec![2.0]);

        let drained = idx.swap_and_drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(idx.pending_count(), 0, "buffer should be empty after drain");
    }

    #[test]
    fn test_deferred_swap_and_drain_clears_deleted_ids() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        idx.remove(1);
        let _drained = idx.swap_and_drain();
        // After drain, deleted_ids should be cleared
        assert!(idx.deleted_ids.read().is_empty());
    }

    #[test]
    fn test_deferred_swap_and_drain_reactivates_buffer() {
        // Regression: swap_and_drain must re-activate the buffer so that
        // pushes between drain and the next merge are not silently dropped.
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        let _ = idx.swap_and_drain();

        // After drain, the buffer should be re-activated and accept pushes.
        idx.push(2, vec![2.0]);
        assert_eq!(idx.pending_count(), 1, "push after drain must succeed");
        assert!(
            idx.is_searchable(),
            "buffer should be searchable after push"
        );
    }

    #[test]
    fn test_deferred_drain_all_leaves_buffer_inactive() {
        // drain_all is for shutdown — buffer is left inactive (not
        // re-activated like swap_and_drain). A subsequent push *will*
        // re-activate via ensure_buffer_active, but there is a window
        // where the buffer is inactive immediately after drain_all.
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        let _ = idx.drain_all();

        // Immediately after drain_all the buffer is inactive.
        assert!(
            !idx.is_searchable(),
            "buffer must not be searchable immediately after drain_all"
        );
        assert_eq!(
            idx.pending_count(),
            0,
            "buffer should be empty after drain_all"
        );
    }

    // ── Merge with HNSW tests ────────────────────────────────────────────

    #[test]
    fn test_deferred_merge_with_hnsw() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(10, vec![0.9, 0.1]);
        idx.push(30, vec![0.5, 0.5]);

        // HNSW results: id=10 (also in buffer) and id=20 (only in HNSW)
        let hnsw = vec![(10, 0.95_f32), (20, 0.80_f32)];
        let merged = idx.merge_with_hnsw(hnsw, &[1.0, 0.0], 3, DistanceMetric::Cosine);

        // No duplicate IDs
        let ids: Vec<u64> = merged.iter().map(|(id, _)| *id).collect();
        let unique: HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "no duplicate IDs");

        // All three IDs should be present (10 from buffer, 20 from HNSW, 30 from buffer)
        assert_eq!(merged.len(), 3);
        assert!(ids.contains(&10));
        assert!(ids.contains(&20));
        assert!(ids.contains(&30));

        // Buffer score for id=10 should be kept (not the HNSW score of 0.95),
        // because the buffer holds more-recent data (upserts route there).
        let id10_score = merged.iter().find(|(id, _)| *id == 10).map(|(_, s)| *s);
        assert!(
            (id10_score.unwrap_or(0.0) - 0.95).abs() > f32::EPSILON,
            "buffer score should be authoritative for id=10, not HNSW"
        );
    }

    #[test]
    fn test_deferred_merge_with_hnsw_empty_buffer() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        // Buffer is empty — merge should return HNSW results unchanged
        let hnsw = vec![(1, 0.9_f32), (2, 0.8_f32)];
        let merged = idx.merge_with_hnsw(hnsw.clone(), &[1.0, 0.0], 5, DistanceMetric::Cosine);
        assert_eq!(merged, hnsw);
    }

    // ── Drain-all test ───────────────────────────────────────────────────

    #[test]
    fn test_deferred_drain_all() {
        let idx = DeferredIndexer::new(enabled_config(1024));
        idx.push(1, vec![1.0]);
        idx.push(2, vec![2.0]);

        let all = idx.drain_all();
        assert_eq!(all.len(), 2);
        assert_eq!(idx.pending_count(), 0);
        assert!(!idx.is_searchable(), "not searchable after drain_all");
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
