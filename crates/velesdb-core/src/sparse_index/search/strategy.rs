//! Search strategy implementations: `MaxScore` DAAT and linear scan fallback.

#![allow(clippy::cast_precision_loss)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rustc_hash::FxHashMap;

use super::super::inverted_index::SparseInvertedIndex;
use super::super::types::{PostingEntry, ScoredDoc, SparseVector};
use super::scoring::{
    extract_sorted_results, find_min_essential_doc_id, find_split, prepare_term_data,
    score_document, PreparedTerms,
};
use super::MAX_DENSE_ACCUMULATOR;

/// `MaxScore` DAAT search over the inverted index.
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

    let mut heap: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    let mut threshold: f32 = 0.0;
    let mut split = find_split(&upper_bound, threshold);
    let mut cursors: Vec<usize> = vec![0; term_data.len()];

    loop {
        let Some(doc_id) = find_min_essential_doc_id(&term_data, &cursors, split) else {
            break;
        };

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
        linear_scan_dense(k, max_doc_id, &term_postings)
    } else {
        linear_scan_hashmap(k, &term_postings)
    }
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
) -> Vec<ScoredDoc> {
    #[allow(clippy::cast_possible_truncation)]
    let size = (max_doc_id + 1) as usize;
    let mut scores = vec![0.0_f32; size];
    let mut touched: Vec<u64> = Vec::new();

    for (qw, postings) in term_postings {
        for entry in postings {
            #[allow(clippy::cast_possible_truncation)]
            let idx = entry.doc_id as usize;
            if scores[idx] == 0.0 {
                touched.push(entry.doc_id);
            }
            scores[idx] += qw * entry.weight;
        }
    }

    let mut heap: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    for &doc_id in &touched {
        #[allow(clippy::cast_possible_truncation)]
        let score = scores[doc_id as usize];
        topk_push(&mut heap, k, doc_id, score);
    }

    extract_sorted_results(heap)
}

/// Hash map accumulator variant for linear scan (large doc ID space).
fn linear_scan_hashmap(k: usize, term_postings: &[(f32, Vec<PostingEntry>)]) -> Vec<ScoredDoc> {
    let mut scores: FxHashMap<u64, f32> = FxHashMap::default();

    for (qw, postings) in term_postings {
        for entry in postings {
            *scores.entry(entry.doc_id).or_insert(0.0) += qw * entry.weight;
        }
    }

    let mut heap: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    for (&doc_id, &score) in &scores {
        topk_push(&mut heap, k, doc_id, score);
    }

    extract_sorted_results(heap)
}
