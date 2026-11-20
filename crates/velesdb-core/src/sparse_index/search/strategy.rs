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

/// Decides, per document id, whether a scored document may be returned.
pub(crate) type IdFilter<'a> = &'a dyn Fn(u64) -> bool;

/// Linear scan: scores every document sharing a term with the query and
/// returns the best `k` — of those `filter` keeps, when there is one.
///
/// The filter is applied to the scored set itself, so the result is exact
/// however selective it is: a filter matching ten documents out of a million
/// returns those ten, in score order.
pub(crate) fn linear_scan_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
    filter: Option<IdFilter<'_>>,
) -> Vec<ScoredDoc> {
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
        return Vec::new();
    }

    // Choose accumulator strategy based on max_doc_id and ID-space density.
    let doc_count = index.doc_count();
    let use_dense = max_doc_id <= MAX_DENSE_ACCUMULATOR
        && (doc_count == 0 || max_doc_id < doc_count.saturating_mul(4));
    if use_dense {
        linear_scan_dense(k, max_doc_id, &term_postings, filter)
    } else {
        linear_scan_hashmap(k, &term_postings, filter)
    }
}

/// The best `k` of `scored`, descending — the selection both accumulator
/// variants end with.
fn select_top_k(
    scored: impl Iterator<Item = ScoredDoc>,
    k: usize,
    filter: Option<IdFilter<'_>>,
) -> Vec<ScoredDoc> {
    let Some(keep) = filter else {
        let mut heap = top_k_heap(k);
        for doc in scored {
            topk_push(&mut heap, k, doc.doc_id, doc.score);
        }
        return extract_sorted_results(heap);
    };
    top_k_kept(scored.collect(), k, keep)
}

/// The best `k` documents `keep` accepts, descending.
///
/// Walks the scored set best-first and stops at the `k`-th accepted document,
/// so `keep` — a payload read per call on the query path — runs only on the
/// documents that outrank the last result, the fewest an exact answer allows.
/// Building the heap is linear in the scored set, like the unfiltered scan.
fn top_k_kept(scored: Vec<ScoredDoc>, k: usize, keep: IdFilter<'_>) -> Vec<ScoredDoc> {
    let mut best_first = BinaryHeap::from(scored);
    let mut kept = Vec::with_capacity(k.min(MAX_HEAP_PREALLOCATION));
    while kept.len() < k {
        let Some(doc) = best_first.pop() else { break };
        if keep(doc.doc_id) {
            kept.push(doc);
        }
    }
    kept
}

/// Pushes a scored document into a min-heap of size `k`, evicting the minimum
/// when full. This is the shared top-k selection primitive for both
/// accumulator variants.
#[inline]
fn topk_push(heap: &mut BinaryHeap<Reverse<ScoredDoc>>, k: usize, doc_id: u64, score: f32) {
    if heap.len() < k {
        heap.push(Reverse(ScoredDoc { score, doc_id }));
    } else if let Some(Reverse(min)) = heap.peek() {
        if score > min.score {
            heap.pop();
            heap.push(Reverse(ScoredDoc { score, doc_id }));
        }
    }
}

/// Dense accumulator variant for linear scan (small doc ID space).
fn linear_scan_dense(
    k: usize,
    max_doc_id: u64,
    term_postings: &[(f32, Vec<PostingEntry>)],
    filter: Option<IdFilter<'_>>,
) -> Vec<ScoredDoc> {
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

    let candidates = touched.iter().map(|&doc_id| {
        #[allow(clippy::cast_possible_truncation)]
        let score = scores[doc_id as usize];
        ScoredDoc { score, doc_id }
    });
    select_top_k(candidates, k, filter)
}

/// Hash map accumulator variant for linear scan (large doc ID space).
fn linear_scan_hashmap(
    k: usize,
    term_postings: &[(f32, Vec<PostingEntry>)],
    filter: Option<IdFilter<'_>>,
) -> Vec<ScoredDoc> {
    let mut scores: FxHashMap<u64, f32> = FxHashMap::default();

    for (qw, postings) in term_postings {
        count_ops(postings.len() as u64);
        for entry in postings {
            *scores.entry(entry.doc_id).or_insert(0.0) += qw * entry.weight;
        }
    }

    let candidates = scores
        .iter()
        .map(|(&doc_id, &score)| ScoredDoc { score, doc_id });
    select_top_k(candidates, k, filter)
}

#[cfg(test)]
#[path = "strategy_tests.rs"]
mod tests;
