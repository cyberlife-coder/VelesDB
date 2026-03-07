//! Delta buffer for accumulating vectors during HNSW rebuilds.
//!
//! The [`DeltaBuffer`] holds recently inserted vectors that have not yet been
//! indexed into the HNSW graph (e.g., because a rebuild is in progress).
//! The search pipeline brute-force scans this buffer and merges results with
//! HNSW results for immediate searchability via [`merge_with_delta`].
//!
//! # Lock ordering
//!
//! `DeltaBuffer` is at position **10** in the collection lock order
//! (after `sparse_indexes` at 9). Code must never hold a delta buffer lock
//! while acquiring a lower-numbered lock.

use crate::distance::DistanceMetric;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

/// Delta buffer for streaming inserts during HNSW rebuilds.
///
/// Accumulates `(point_id, vector)` pairs that are in storage but not yet in
/// the HNSW index. When active, search methods brute-force scan the buffer
/// and merge results with HNSW results via [`merge_with_delta`].
pub struct DeltaBuffer {
    /// Buffered `(point_id, vector)` pairs awaiting index insertion.
    points: RwLock<Vec<(u64, Vec<f32>)>>,

    /// Whether the delta buffer is actively accumulating (rebuild in progress).
    active: AtomicBool,
}

impl DeltaBuffer {
    /// Creates an empty, inactive delta buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            points: RwLock::new(Vec::new()),
            active: AtomicBool::new(false),
        }
    }

    /// Returns `true` if the delta buffer is actively accumulating vectors
    /// (i.e., an HNSW rebuild is in progress).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Activates the delta buffer (marks a rebuild as in progress).
    ///
    /// While active, the drain loop will push vectors into this buffer so
    /// that search can find them before they are indexed into HNSW.
    pub fn activate(&self) {
        self.active.store(true, Ordering::Release);
    }

    /// Deactivates the buffer and drains all buffered points.
    ///
    /// Returns the accumulated `(point_id, vector)` pairs for progressive
    /// merge into the newly rebuilt HNSW index. After this call, the buffer
    /// is empty and inactive.
    pub fn deactivate_and_drain(&self) -> Vec<(u64, Vec<f32>)> {
        let mut points = self.points.write();
        self.active.store(false, Ordering::Release);
        std::mem::take(&mut *points)
    }

    /// Pushes a single entry into the delta buffer.
    pub fn push(&self, id: u64, vector: Vec<f32>) {
        self.points.write().push((id, vector));
    }

    /// Extends the delta buffer with multiple entries.
    pub fn extend(&self, entries: impl IntoIterator<Item = (u64, Vec<f32>)>) {
        self.points.write().extend(entries);
    }

    /// Returns the number of buffered entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.read().len()
    }

    /// Returns `true` if the buffer contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.read().is_empty()
    }

    /// Brute-force searches the delta buffer for the k nearest neighbors.
    ///
    /// Returns an empty `Vec` if the buffer is not active. Otherwise computes
    /// distances using `metric.calculate()` (which dispatches to SIMD kernels),
    /// sorts by the metric's ordering, and truncates to `k`.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, metric: DistanceMetric) -> Vec<(u64, f32)> {
        if !self.is_active() {
            return Vec::new();
        }

        let points = self.points.read();
        let mut results: Vec<(u64, f32)> = points
            .iter()
            .map(|(id, vec)| (*id, metric.calculate(query, vec)))
            .collect();

        metric.sort_results(&mut results);
        results.truncate(k);
        results
    }
}

impl Default for DeltaBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Merges HNSW search results with delta buffer brute-force results.
///
/// If the delta buffer is not active, returns `hnsw_results` unchanged.
/// Otherwise, performs a brute-force scan of the delta, deduplicates by ID
/// (delta entries win on conflict since they may be more recent), sorts by
/// the metric's ordering, and truncates to `k`.
#[must_use]
pub fn merge_with_delta(
    hnsw_results: Vec<(u64, f32)>,
    delta: &DeltaBuffer,
    query: &[f32],
    k: usize,
    metric: DistanceMetric,
) -> Vec<(u64, f32)> {
    if !delta.is_active() {
        return hnsw_results;
    }

    let delta_results = delta.search(query, k, metric);
    if delta_results.is_empty() {
        return hnsw_results;
    }

    // Delta IDs win on duplicates (more recent data).
    let delta_ids: HashSet<u64> = delta_results.iter().map(|(id, _)| *id).collect();
    let mut merged: Vec<(u64, f32)> = hnsw_results
        .into_iter()
        .filter(|(id, _)| !delta_ids.contains(id))
        .collect();
    merged.extend(delta_results);

    metric.sort_results(&mut merged);
    merged.truncate(k);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_delta_buffer_compiles_and_defaults_inactive() {
        let buf = DeltaBuffer::new();
        assert!(
            !buf.is_active(),
            "new DeltaBuffer should be inactive by default"
        );
    }

    #[test]
    fn test_stream_delta_buffer_default_trait() {
        let buf = DeltaBuffer::default();
        assert!(!buf.is_active());
    }

    #[test]
    fn test_stream_delta_push_and_search() {
        let buf = DeltaBuffer::new();
        buf.activate();
        buf.push(1, vec![1.0, 0.0, 0.0]);
        buf.push(2, vec![0.0, 1.0, 0.0]);
        buf.push(3, vec![0.5, 0.5, 0.0]);

        let query = &[1.0, 0.0, 0.0];
        let results = buf.search(query, 2, DistanceMetric::Cosine);
        assert_eq!(results.len(), 2, "should return at most k=2 results");
        // Cosine: higher is better; [1,0,0] is identical to query -> highest score
        assert_eq!(
            results[0].0, 1,
            "closest match should be id=1 (identical vector)"
        );
    }

    #[test]
    fn test_stream_delta_search_returns_empty_when_inactive() {
        let buf = DeltaBuffer::new();
        buf.push(1, vec![1.0, 0.0, 0.0]);
        // buffer is NOT active
        let results = buf.search(&[1.0, 0.0, 0.0], 10, DistanceMetric::Cosine);
        assert!(
            results.is_empty(),
            "inactive delta should return no results"
        );
    }

    #[test]
    fn test_stream_delta_search_cosine_ordering() {
        let buf = DeltaBuffer::new();
        buf.activate();
        // Vec pointing along x-axis
        buf.push(10, vec![1.0, 0.0]);
        // Vec pointing along y-axis (orthogonal)
        buf.push(20, vec![0.0, 1.0]);
        // Vec at 45 degrees
        buf.push(30, vec![1.0, 1.0]);

        let query = &[1.0, 0.0];
        let results = buf.search(query, 3, DistanceMetric::Cosine);
        // Cosine: higher is better. id=10 should be first (similarity ~1.0)
        assert_eq!(results[0].0, 10);
        // id=30 at 45 deg should be next (similarity ~0.707)
        assert_eq!(results[1].0, 30);
        // id=20 orthogonal should be last (similarity ~0.0)
        assert_eq!(results[2].0, 20);
    }

    #[test]
    fn test_stream_delta_search_euclidean_ordering() {
        let buf = DeltaBuffer::new();
        buf.activate();
        buf.push(1, vec![0.0, 0.0]);
        buf.push(2, vec![1.0, 0.0]);
        buf.push(3, vec![3.0, 4.0]);

        let query = &[0.0, 0.0];
        let results = buf.search(query, 3, DistanceMetric::Euclidean);
        // Euclidean: lower is better. id=1 (dist=0) should be first
        assert_eq!(results[0].0, 1);
        assert_eq!(results[1].0, 2);
        assert_eq!(results[2].0, 3);
    }

    #[test]
    fn test_stream_delta_merge_with_delta_inactive() {
        let buf = DeltaBuffer::new();
        // NOT active
        let hnsw = vec![(1, 0.9), (2, 0.8)];
        let merged = merge_with_delta(hnsw.clone(), &buf, &[1.0, 0.0], 5, DistanceMetric::Cosine);
        assert_eq!(merged, hnsw, "inactive delta should return HNSW unchanged");
    }

    #[test]
    fn test_stream_delta_merge_dedup_and_truncate() {
        let buf = DeltaBuffer::new();
        buf.activate();
        // Delta has id=1 with a different score and id=3 (new)
        buf.push(1, vec![0.9, 0.1]);
        buf.push(3, vec![0.8, 0.2]);

        // HNSW results (cosine scores, higher is better)
        let hnsw = vec![(1, 0.95), (2, 0.80)];

        let query = &[1.0, 0.0];
        let merged = merge_with_delta(hnsw, &buf, query, 2, DistanceMetric::Cosine);

        // Should have at most k=2 results
        assert_eq!(merged.len(), 2);

        // Delta wins for id=1 — its score should come from delta's brute-force
        // Check no duplicate ids
        let ids: Vec<u64> = merged.iter().map(|(id, _)| *id).collect();
        let unique: HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "no duplicate IDs in merged results"
        );
    }

    #[test]
    fn test_stream_delta_merge_empty_delta() {
        let buf = DeltaBuffer::new();
        buf.activate();
        // Delta is active but empty
        let hnsw = vec![(1, 0.9), (2, 0.8)];
        let merged = merge_with_delta(hnsw.clone(), &buf, &[1.0, 0.0], 5, DistanceMetric::Cosine);
        assert_eq!(
            merged, hnsw,
            "empty active delta should return HNSW unchanged"
        );
    }

    #[test]
    fn test_stream_delta_activate_deactivate_drain() {
        let buf = DeltaBuffer::new();
        assert!(!buf.is_active());

        buf.activate();
        assert!(buf.is_active());

        buf.push(1, vec![1.0]);
        buf.push(2, vec![2.0]);
        assert_eq!(buf.len(), 2);

        let drained = buf.deactivate_and_drain();
        assert!(!buf.is_active());
        assert!(buf.is_empty());
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, 1);
        assert_eq!(drained[1].0, 2);
    }

    #[test]
    fn test_stream_delta_extend() {
        let buf = DeltaBuffer::new();
        buf.extend(vec![(1, vec![1.0]), (2, vec![2.0]), (3, vec![3.0])]);
        assert_eq!(buf.len(), 3);
    }
}
