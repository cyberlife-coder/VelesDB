//! Scoring primitives for sparse search.
//!
//! Contains term-posting structures, upper-bound preparation, per-document
//! scoring, and result extraction helpers shared by all search strategies.

#![allow(clippy::cast_precision_loss)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::super::inverted_index::SparseInvertedIndex;
use super::super::types::{PostingEntry, ScoredDoc, SparseVector};

/// Collects posting lists for each query term, along with query weight and
/// the global max document weight for that term.
pub(crate) struct TermPostings {
    pub query_weight: f32,
    pub max_doc_weight: f32,
    pub postings: Vec<PostingEntry>,
}

/// Prepared term data with precomputed upper-bound prefix sums for `MaxScore`.
pub(crate) struct PreparedTerms {
    pub terms: Vec<TermPostings>,
    /// `upper_bound[i]` = cumulative max contribution of terms `0..=i`.
    pub upper_bound: Vec<f32>,
}

/// Collects posting lists for each query term, sorts by max contribution
/// ascending, and computes upper-bound prefix sums.
///
/// Returns `None` when no query term has any postings.
pub(crate) fn prepare_term_data(
    index: &SparseInvertedIndex,
    query: &SparseVector,
) -> Option<PreparedTerms> {
    let mut terms: Vec<TermPostings> = Vec::with_capacity(query.nnz());
    for (&term_id, &qw) in query.indices.iter().zip(query.values.iter()) {
        let postings = index.get_all_postings(term_id);
        if postings.is_empty() {
            continue;
        }
        let max_dw = index.get_global_max_weight(term_id);
        terms.push(TermPostings {
            query_weight: qw,
            max_doc_weight: max_dw,
            postings,
        });
    }

    if terms.is_empty() {
        return None;
    }

    // Sort by max_contribution ascending (least contributing first)
    terms.sort_unstable_by(|a, b| {
        let ca = a.query_weight.abs() * a.max_doc_weight;
        let cb = b.query_weight.abs() * b.max_doc_weight;
        ca.total_cmp(&cb)
    });

    // Compute prefix sums of max contributions (upper_bound[i] = sum of 0..=i)
    let n = terms.len();
    let mut upper_bound = vec![0.0_f32; n];
    upper_bound[0] = terms[0].query_weight.abs() * terms[0].max_doc_weight;
    for i in 1..n {
        upper_bound[i] = upper_bound[i - 1] + terms[i].query_weight.abs() * terms[i].max_doc_weight;
    }

    Some(PreparedTerms { terms, upper_bound })
}

/// Scores a single document by combining essential term contributions (at
/// cursor positions matching `doc_id`) and non-essential term contributions
/// (via binary search).
pub(crate) fn score_document(
    term_data: &[TermPostings],
    cursors: &mut [usize],
    split: usize,
    doc_id: u64,
) -> f32 {
    let mut score = 0.0_f32;

    // Essential terms: advance cursors that match doc_id
    for i in split..term_data.len() {
        if cursors[i] < term_data[i].postings.len()
            && term_data[i].postings[cursors[i]].doc_id == doc_id
        {
            score += term_data[i].query_weight * term_data[i].postings[cursors[i]].weight;
            cursors[i] += 1;
        }
    }

    // Non-essential terms: binary search for doc_id
    for td in &term_data[..split] {
        if let Ok(pos) = td.postings.binary_search_by_key(&doc_id, |e| e.doc_id) {
            score += td.query_weight * td.postings[pos].weight;
        }
    }

    score
}

/// Finds the smallest `doc_id` among essential posting lists (from `split..n_terms`).
pub(crate) fn find_min_essential_doc_id(
    term_data: &[TermPostings],
    cursors: &[usize],
    split: usize,
) -> Option<u64> {
    let mut min_doc_id: Option<u64> = None;
    for i in split..term_data.len() {
        if cursors[i] < term_data[i].postings.len() {
            let did = term_data[i].postings[cursors[i]].doc_id;
            min_doc_id = Some(min_doc_id.map_or(did, |m: u64| m.min(did)));
        }
    }
    min_doc_id
}

/// Finds the split index: smallest i where `upper_bound[i] >= threshold`.
/// Returns `upper_bound.len()` if no such index exists (all terms are non-essential).
pub(crate) fn find_split(upper_bound: &[f32], threshold: f32) -> usize {
    for (i, &ub) in upper_bound.iter().enumerate() {
        if ub >= threshold {
            return i;
        }
    }
    upper_bound.len()
}

/// Extract results from a min-heap and sort descending by score.
pub(crate) fn extract_sorted_results(heap: BinaryHeap<Reverse<ScoredDoc>>) -> Vec<ScoredDoc> {
    let mut results: Vec<ScoredDoc> = heap.into_iter().map(|Reverse(s)| s).collect();
    results.sort_unstable_by(|a, b| b.cmp(a)); // descending
    results
}
