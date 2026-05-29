//! Bounded top-k accumulator for the scan/score path (#901).
//!
//! Collects the `k` best [`SearchResult`]s by score **without** materializing
//! the full candidate set. Memory is bounded to `O(k)` rather than `O(n)`,
//! while the returned results and their ordering are identical to a full sort
//! followed by `truncate(k)`.
//!
//! The accumulator keeps a binary heap whose *root is the worst* result kept so
//! far (for the current metric direction). Once the heap is full, a new
//! candidate only displaces the root when it is strictly better — exactly the
//! semantics of `sort` + `truncate`, but in bounded memory.

use crate::point::SearchResult;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// One entry kept in the bounded heap, ordered so that the heap's max element
/// is the *worst* result currently retained.
struct HeapEntry {
    /// Insertion order, used as a deterministic tie-breaker so equal-score
    /// results keep first-seen order (matching `sort_unstable` stability for
    /// the scan path, where input order is the storage id order).
    seq: u64,
    result: SearchResult,
    higher_is_better: bool,
}

impl HeapEntry {
    /// Orders two entries from *best to worst*. The heap inverts this so the
    /// root is the worst kept entry and can be popped when displaced.
    fn cmp_best_to_worst(&self, other: &Self) -> Ordering {
        let by_score = if self.higher_is_better {
            // Higher score is better → better entry compares as Less ("front").
            other.result.score.total_cmp(&self.result.score)
        } else {
            self.result.score.total_cmp(&other.result.score)
        };
        // Tie-break on insertion order so earlier candidates rank ahead.
        by_score.then(self.seq.cmp(&other.seq))
    }
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp_best_to_worst(other) == Ordering::Equal
    }
}
impl Eq for HeapEntry {}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // `BinaryHeap` is a max-heap and `peek()` returns the greatest element.
        // We want the *worst* kept entry at the root so it can be evicted, so
        // the worst entry must compare as greatest. `cmp_best_to_worst` already
        // ranks the worst entry as `Greater`, so use it directly.
        self.cmp_best_to_worst(other)
    }
}

/// Bounded top-k accumulator. Retains at most `k` best results by score.
pub(super) struct BoundedTopK {
    heap: BinaryHeap<HeapEntry>,
    k: usize,
    higher_is_better: bool,
    next_seq: u64,
}

impl BoundedTopK {
    /// Creates an accumulator that keeps the `k` best results. `higher_is_better`
    /// selects the metric direction (similarity vs. distance).
    pub(super) fn new(k: usize, higher_is_better: bool) -> Self {
        Self {
            // Reserve k+1: we push then pop when over capacity.
            heap: BinaryHeap::with_capacity(k.saturating_add(1).min(4096)),
            k,
            higher_is_better,
            next_seq: 0,
        }
    }

    /// Offers a scored candidate. Kept only if it ranks within the top `k`.
    pub(super) fn offer(&mut self, result: SearchResult) {
        if self.k == 0 {
            return;
        }
        let entry = HeapEntry {
            seq: self.next_seq,
            result,
            higher_is_better: self.higher_is_better,
        };
        self.next_seq += 1;

        if self.heap.len() < self.k {
            self.heap.push(entry);
            return;
        }
        // Heap is full: replace the worst kept entry only if the candidate is
        // strictly better. The heap root is the worst kept entry.
        if let Some(worst) = self.heap.peek() {
            if entry.cmp_best_to_worst(worst) == Ordering::Less {
                self.heap.pop();
                self.heap.push(entry);
            }
        }
    }

    /// Consumes the accumulator, returning results sorted best-to-worst —
    /// identical ordering to a full `sort` + `truncate(k)`.
    pub(super) fn into_sorted_vec(self) -> Vec<SearchResult> {
        let mut entries: Vec<HeapEntry> = self.heap.into_vec();
        entries.sort_unstable_by(HeapEntry::cmp_best_to_worst);
        entries.into_iter().map(|e| e.result).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::Point;

    fn result(id: u64, score: f32) -> SearchResult {
        SearchResult::new(
            Point {
                id,
                vector: vec![],
                payload: None,
                sparse_vectors: None,
            },
            score,
        )
    }

    /// Bounded top-k returns the same ids/order as full sort+truncate
    /// (higher-is-better direction).
    #[test]
    fn test_bounded_top_k_matches_full_sort_higher_better() {
        let scores = [0.1f32, 0.9, 0.5, 0.95, 0.3, 0.7];
        let mut topk = BoundedTopK::new(3, true);
        for (i, s) in scores.iter().enumerate() {
            topk.offer(result(i as u64, *s));
        }
        let got: Vec<(u64, f32)> = topk
            .into_sorted_vec()
            .iter()
            .map(|r| (r.point.id, r.score))
            .collect();

        // Reference: full sort desc + truncate(3).
        let mut reference: Vec<(u64, f32)> = scores
            .iter()
            .enumerate()
            .map(|(i, s)| (i as u64, *s))
            .collect();
        reference.sort_by(|a, b| b.1.total_cmp(&a.1));
        reference.truncate(3);

        assert_eq!(got, reference);
    }

    /// Lower-is-better (distance) direction keeps the smallest scores.
    #[test]
    fn test_bounded_top_k_matches_full_sort_lower_better() {
        let scores = [5.0f32, 1.0, 3.0, 0.5, 9.0, 2.0];
        let mut topk = BoundedTopK::new(2, false);
        for (i, s) in scores.iter().enumerate() {
            topk.offer(result(i as u64, *s));
        }
        let got: Vec<u64> = topk.into_sorted_vec().iter().map(|r| r.point.id).collect();
        assert_eq!(got, vec![3, 1]); // 0.5 then 1.0
    }

    /// Equal scores keep first-seen (insertion) order.
    #[test]
    fn test_bounded_top_k_ties_keep_insertion_order() {
        let mut topk = BoundedTopK::new(2, true);
        topk.offer(result(10, 0.5));
        topk.offer(result(20, 0.5));
        topk.offer(result(30, 0.5));
        let got: Vec<u64> = topk.into_sorted_vec().iter().map(|r| r.point.id).collect();
        assert_eq!(got, vec![10, 20]);
    }
}
