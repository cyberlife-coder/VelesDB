//! Search strategy implementations: `MaxScore` DAAT and linear scan fallback.

#![allow(clippy::cast_precision_loss)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rustc_hash::FxHashMap;

use super::super::inverted_index::SparseInvertedIndex;
use super::super::types::{PostingEntry, ScoredDoc, SparseVector};
use super::scoring::{count_ops, extract_sorted_results};
#[cfg(any(test, feature = "internal-bench"))]
use super::scoring::{
    find_min_essential_doc_id, find_split, prepare_term_data, score_document, PreparedTerms,
};
use super::MAX_DENSE_ACCUMULATOR;

/// Most heap entries reserved up front, whatever `k` the caller sent.
const MAX_HEAP_PREALLOCATION: usize = 4096;

/// An empty top-`k` heap that reserves `min(k + 1, MAX_HEAP_PREALLOCATION)`.
///
/// `k` is the caller's top-k and reaches this module unclamped. Reserving
/// `k + 1` entries made a huge `k` abort the whole process on allocation
/// failure and `usize::MAX` overflow the `+ 1`. The heap never holds more than
/// the scored documents, so the reservation is only an optimisation: past the
/// cap it grows as results arrive.
fn top_k_heap(k: usize) -> BinaryHeap<Reverse<ScoredDoc>> {
    BinaryHeap::with_capacity(k.saturating_add(1).min(MAX_HEAP_PREALLOCATION))
}

/// `MaxScore` DAAT search over the inverted index.
///
/// Retired from production routing (#2177 — measured strictly dominated at
/// every corpus size; the figures live in the `search` module doc);
/// reachable only from tests and the `internal-bench` seam for
/// re-measurement.
#[cfg(any(test, feature = "internal-bench"))]
pub(crate) fn maxscore_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<ScoredDoc> {
    let Some(prepared) = prepare_term_data(index, query) else {
        return Vec::new();
    };
    let PreparedTerms {
        terms: term_data,
        upper_bound,
    } = prepared;

    let mut heap = top_k_heap(k);
    let mut threshold: f32 = 0.0;
    let mut split = find_split(&upper_bound, threshold);
    let mut cursors: Vec<usize> = vec![0; term_data.len()];

    while let Some(doc_id) = find_min_essential_doc_id(&term_data, &cursors, split) {
        let score = score_document(&term_data, &mut cursors, split, doc_id);

        if heap.len() < k || score > threshold {
            heap.push(Reverse(ScoredDoc { score, doc_id }));
            if heap.len() > k {
                heap.pop();
            }
            if heap.len() == k {
                threshold = heap.peek().map_or(0.0, |Reverse(s)| s.score);
                split = find_split(&upper_bound, threshold);
            }
        }
    }

    extract_sorted_results(heap)
}

/// Linear scan fallback for high-coverage queries.
pub(crate) fn linear_scan_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<ScoredDoc> {
    let Some(scan) = PostingScan::of(index, query) else {
        return Vec::new();
    };
    if scan.use_dense {
        linear_scan_dense(k, scan.max_doc_id, &scan.term_postings)
    } else {
        linear_scan_hashmap(k, &scan.term_postings)
    }
}

/// Hands `emit` every document the linear scan scores for `query`, as
/// `(doc_id, inner product)`, each once, in no particular order: what
/// [`linear_scan_search`] cuts to its top `k`, for a caller that decides
/// which documents may take a place before the cut
/// (`collection/search/ranked.rs`, [`super::sparse_search_filtered`]).
pub(crate) fn linear_scan_scores(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    mut emit: impl FnMut(u64, f32),
) {
    let Some(scan) = PostingScan::of(index, query) else {
        return;
    };
    if scan.use_dense {
        let (scores, touched) = accumulate_dense(scan.max_doc_id, &scan.term_postings);
        for doc_id in touched {
            #[allow(clippy::cast_possible_truncation)]
            emit(doc_id, scores[doc_id as usize]);
        }
    } else {
        for (doc_id, score) in accumulate_hashmap(&scan.term_postings) {
            emit(doc_id, score);
        }
    }
}

/// The posting lists of a query's terms, and the accumulator they fit.
struct PostingScan {
    term_postings: Vec<(f32, Vec<PostingEntry>)>,
    max_doc_id: u64,
    use_dense: bool,
}

impl PostingScan {
    /// The posting lists of `query`'s terms in `index`; `None` when no term
    /// has a posting.
    fn of(index: &SparseInvertedIndex, query: &SparseVector) -> Option<Self> {
        // Collect all posting lists and find max doc_id
        let mut term_postings: Vec<(f32, Vec<PostingEntry>)> = Vec::with_capacity(query.nnz());
        let mut max_doc_id: u64 = 0;

        for (&term_id, &qw) in query.indices.iter().zip(query.values.iter()) {
            let postings = index.get_all_postings(term_id);
            if postings.is_empty() {
                continue;
            }
            if let Some(last) = postings.last() {
                max_doc_id = max_doc_id.max(last.doc_id);
            }
            term_postings.push((qw, postings));
        }

        if term_postings.is_empty() {
            return None;
        }

        // Choose accumulator strategy based on max_doc_id and ID-space density.
        let doc_count = index.doc_count();
        let use_dense = max_doc_id <= MAX_DENSE_ACCUMULATOR
            && (doc_count == 0 || max_doc_id < doc_count.saturating_mul(4));
        Some(Self {
            term_postings,
            max_doc_id,
            use_dense,
        })
    }
}

/// Pushes a scored document into a min-heap of size `k`, evicting the minimum
/// when full and the document ranks above it ([`ScoredDoc`]'s order, so a
/// tie is decided by id, not by the order the accumulator visits the
/// documents in). This is the shared top-k selection primitive for both
/// accumulator variants.
#[inline]
fn topk_push(heap: &mut BinaryHeap<Reverse<ScoredDoc>>, k: usize, doc_id: u64, score: f32) {
    let doc = ScoredDoc { score, doc_id };
    if heap.len() < k {
        heap.push(Reverse(doc));
    } else if heap.peek().is_some_and(|Reverse(min)| doc > *min) {
        heap.pop();
        heap.push(Reverse(doc));
    }
}

/// Dense accumulator variant for linear scan (small doc ID space).
fn linear_scan_dense(
    k: usize,
    max_doc_id: u64,
    term_postings: &[(f32, Vec<PostingEntry>)],
) -> Vec<ScoredDoc> {
    let (scores, touched) = accumulate_dense(max_doc_id, term_postings);
    let mut heap = top_k_heap(k);
    for &doc_id in &touched {
        #[allow(clippy::cast_possible_truncation)]
        let score = scores[doc_id as usize];
        topk_push(&mut heap, k, doc_id, score);
    }

    extract_sorted_results(heap)
}

/// The dense accumulator's scores, indexed by doc id, and the doc ids the
/// postings touched, each once, in the order first touched.
fn accumulate_dense(
    max_doc_id: u64,
    term_postings: &[(f32, Vec<PostingEntry>)],
) -> (Vec<f32>, Vec<u64>) {
    #[allow(clippy::cast_possible_truncation)]
    let size = (max_doc_id + 1) as usize;
    let mut scores = vec![0.0_f32; size];
    // Membership is a fact about the scan, not about the value: a negative
    // query weight can drive an accumulated score back to exactly `0.0`, so
    // testing `scores[idx] == 0.0` would record the same document twice and
    // let the duplicate evict a genuine result from the top-k heap.
    let mut seen = vec![false; size];
    let mut touched: Vec<u64> = Vec::new();

    for (qw, postings) in term_postings {
        // The whole of this strategy's scoring cost: one inspection per
        // posting, against MaxScore's terms x log(postings) per candidate.
        count_ops(postings.len() as u64);
        for entry in postings {
            #[allow(clippy::cast_possible_truncation)]
            let idx = entry.doc_id as usize;
            if !std::mem::replace(&mut seen[idx], true) {
                touched.push(entry.doc_id);
            }
            scores[idx] += qw * entry.weight;
        }
    }
    (scores, touched)
}

/// Hash map accumulator variant for linear scan (large doc ID space).
fn linear_scan_hashmap(k: usize, term_postings: &[(f32, Vec<PostingEntry>)]) -> Vec<ScoredDoc> {
    let scores = accumulate_hashmap(term_postings);
    let mut heap = top_k_heap(k);
    for (&doc_id, &score) in &scores {
        topk_push(&mut heap, k, doc_id, score);
    }

    extract_sorted_results(heap)
}

/// The hash map accumulator's score of every doc id the postings touched.
fn accumulate_hashmap(term_postings: &[(f32, Vec<PostingEntry>)]) -> FxHashMap<u64, f32> {
    let mut scores: FxHashMap<u64, f32> = FxHashMap::default();

    for (qw, postings) in term_postings {
        count_ops(postings.len() as u64);
        for entry in postings {
            *scores.entry(entry.doc_id).or_insert(0.0) += qw * entry.weight;
        }
    }
    scores
}

#[cfg(test)]
#[path = "strategy_tests.rs"]
mod tests;
