//! Deferred indexer for high-throughput sequential vector inserts.
//!
//! The [`DeferredIndexer`] buffers incoming vectors in memory and exposes them
//! to search via brute-force scan while they await insertion into the HNSW
//! graph. This decouples the write path (fast, O(1) per point) from the index
//! path (slower, O(log n) per point) and enables background merge.
//!
//! # Double-buffering
//!
//! Internally the indexer holds a *front* buffer that accepts writes and a
//! *back* buffer used during drain. [`swap_and_drain`](DeferredIndexer::swap_and_drain)
//! rotates front to back, drains the old front, and returns the vectors for
//! the caller to insert into HNSW.
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
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
    /// Front buffer — accepts writes.
    front: Arc<DeltaBuffer>,

    /// Back buffer — used during swap-and-drain.
    back: Arc<DeltaBuffer>,

    /// Serializes swap-and-drain operations so only one drain runs at a time.
    swap_lock: Mutex<()>,

    /// IDs deleted while in the buffer. Filtered out of search results.
    deleted_ids: RwLock<HashSet<u64>>,

    /// Configuration (immutable after construction).
    config: DeferredIndexerConfig,
}

impl DeferredIndexer {
    /// Creates a new `DeferredIndexer` with the given configuration.
    ///
    /// Both buffers start inactive. If `config.enabled` is `false`, all
    /// write operations are no-ops.
    #[must_use]
    pub fn new(config: DeferredIndexerConfig) -> Self {
        Self {
            front: Arc::new(DeltaBuffer::new()),
            back: Arc::new(DeltaBuffer::new()),
            swap_lock: Mutex::new(()),
            deleted_ids: RwLock::new(HashSet::new()),
            config,
        }
    }

    /// Whether deferred indexing is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Pushes a vector into the front buffer.
    ///
    /// Activates the front buffer lazily on first write. Returns `true` if
    /// the front buffer has reached `merge_threshold`, signaling the caller
    /// to trigger a merge.
    ///
    /// No-op if deferred indexing is disabled.
    pub fn push(&self, id: u64, vector: Vec<f32>) -> bool {
        if !self.config.enabled {
            return false;
        }
        self.ensure_front_active();
        self.front.push(id, vector);
        self.front.len() >= self.config.merge_threshold
    }

    /// Batch-pushes vectors into the front buffer.
    ///
    /// Returns `true` if the front buffer has reached `merge_threshold`.
    /// No-op if deferred indexing is disabled.
    pub fn extend(&self, entries: impl IntoIterator<Item = (u64, Vec<f32>)>) -> bool {
        if !self.config.enabled {
            return false;
        }
        self.ensure_front_active();
        self.front.extend(entries);
        self.front.len() >= self.config.merge_threshold
    }

    /// Marks `id` as deleted, removing it from both buffers.
    ///
    /// The ID is added to `deleted_ids` so that search results are filtered
    /// even if the vector was already snapshot for a concurrent search.
    pub fn remove(&self, id: u64) {
        self.front.remove(id);
        self.back.remove(id);
        self.deleted_ids.write().insert(id);
    }

    /// Brute-force searches both buffers, filtering deleted IDs.
    ///
    /// Results are deduplicated by ID (best score wins), sorted by the
    /// metric ordering, and truncated to `k`.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, metric: DistanceMetric) -> Vec<(u64, f32)> {
        let front_results = self.front.search(query, k, metric);
        let back_results = self.back.search(query, k, metric);

        let deleted = self.deleted_ids.read();
        let merged = merge_and_dedup(front_results, back_results, &deleted, metric);
        truncated(merged, k)
    }

    /// Merges HNSW results with deferred buffer results.
    ///
    /// HNSW is authoritative: if the same ID appears in both HNSW and the
    /// buffer, the HNSW score is kept (the indexed position is canonical).
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
        let hnsw_ids: HashSet<u64> = hnsw_results.iter().map(|(id, _)| *id).collect();
        let mut combined: Vec<(u64, f32)> = hnsw_results;
        combined.extend(
            buffer_results
                .into_iter()
                .filter(|(id, _)| !hnsw_ids.contains(id)),
        );
        metric.sort_results(&mut combined);
        combined.truncate(k);
        combined
    }

    /// Drains the front buffer and returns vectors for HNSW insertion.
    ///
    /// After this call the front buffer is empty and inactive (a subsequent
    /// `push` will re-activate it). The `deleted_ids` set is cleared because
    /// the caller is expected to apply deletions to HNSW after merge.
    ///
    /// Serialized by an internal mutex so concurrent calls are safe (the
    /// second caller gets an empty drain).
    pub fn swap_and_drain(&self) -> Vec<(u64, Vec<f32>)> {
        let _guard = self.swap_lock.lock();
        let drained = self.front.deactivate_and_drain();
        self.deleted_ids.write().clear();
        drained
    }

    /// Total number of pending (not yet indexed) vectors across both buffers.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.front.len() + self.back.len()
    }

    /// Returns `true` if the front buffer has reached `merge_threshold`.
    #[must_use]
    pub fn should_merge(&self) -> bool {
        self.front.len() >= self.config.merge_threshold
    }

    /// Returns `true` if deferred indexing is enabled and either buffer has
    /// searchable data.
    #[must_use]
    pub fn is_searchable(&self) -> bool {
        self.config.enabled && (self.front.is_searchable() || self.back.is_searchable())
    }

    /// Drains all vectors from both buffers (for shutdown / flush).
    ///
    /// Clears `deleted_ids`. After this call both buffers are empty and
    /// inactive.
    pub fn drain_all(&self) -> Vec<(u64, Vec<f32>)> {
        let _guard = self.swap_lock.lock();
        let mut all = self.front.deactivate_and_drain();
        all.extend(self.back.deactivate_and_drain());
        self.deleted_ids.write().clear();
        all
    }

    /// Lazily activates the front buffer if it is not already active.
    fn ensure_front_active(&self) {
        if !self.front.is_active() {
            self.front.activate();
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Merges two result sets, deduplicating by ID (best score wins) and
/// filtering deleted IDs.
fn merge_and_dedup(
    a: Vec<(u64, f32)>,
    b: Vec<(u64, f32)>,
    deleted: &HashSet<u64>,
    metric: DistanceMetric,
) -> Vec<(u64, f32)> {
    let mut seen: HashSet<u64> = HashSet::with_capacity(a.len() + b.len());
    let mut merged: Vec<(u64, f32)> = Vec::with_capacity(a.len() + b.len());

    for (id, score) in a.into_iter().chain(b) {
        if deleted.contains(&id) {
            continue;
        }
        if seen.insert(id) {
            merged.push((id, score));
        } else {
            update_best_score(&mut merged, id, score, metric);
        }
    }

    metric.sort_results(&mut merged);
    merged
}

/// Updates the score for `id` in `results` if `new_score` is better
/// according to the metric ordering.
fn update_best_score(results: &mut [(u64, f32)], id: u64, new_score: f32, metric: DistanceMetric) {
    for entry in results.iter_mut() {
        if entry.0 == id {
            let keep_new = if metric.higher_is_better() {
                new_score > entry.1
            } else {
                new_score < entry.1
            };
            if keep_new {
                entry.1 = new_score;
            }
            return;
        }
    }
}

/// Truncates a result vector to at most `k` elements.
fn truncated(mut v: Vec<(u64, f32)>, k: usize) -> Vec<(u64, f32)> {
    v.truncate(k);
    v
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
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
        assert_eq!(idx.pending_count(), 0, "front should be empty after drain");
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

        // All three IDs should be present (10 from HNSW, 20 from HNSW, 30 from buffer)
        assert_eq!(merged.len(), 3);
        assert!(ids.contains(&10));
        assert!(ids.contains(&20));
        assert!(ids.contains(&30));

        // HNSW score for id=10 should be kept (0.95), not buffer score
        let id10_score = merged.iter().find(|(id, _)| *id == 10).map(|(_, s)| *s);
        assert!(
            (id10_score.unwrap_or(0.0) - 0.95).abs() < f32::EPSILON,
            "HNSW score should be authoritative for id=10"
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
