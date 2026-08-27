//! Parity between the two linear-scan accumulators.
//!
//! `linear_scan_search` picks between a dense array and a hash map purely on
//! the shape of the doc ID space. That choice is a performance decision, so
//! the two variants must be observationally identical: same documents, same
//! scores, for the same posting lists. These tests call both directly on
//! hand-built posting lists so a divergence is attributed to the accumulator
//! rather than to routing.

use crate::sparse_index::types::{PostingEntry, ScoredDoc};

use super::{linear_scan_dense, linear_scan_hashmap};

fn entry(doc_id: u64, weight: f32) -> PostingEntry {
    PostingEntry { doc_id, weight }
}

fn as_pairs(results: &[ScoredDoc]) -> Vec<(u64, f32)> {
    results.iter().map(|r| (r.doc_id, r.score)).collect()
}

/// Query weights `+1` then `-1` against equal document weights drive the
/// accumulated score for doc 0 back to exactly `0.0` before the third term
/// contributes. Realistic for any "like A but not B" query built from two
/// document vectors that share a term at the same weight.
fn cancelling_term_postings() -> Vec<(f32, Vec<PostingEntry>)> {
    vec![
        (1.0, vec![entry(0, 1.0), entry(1, 5.0)]),
        (-1.0, vec![entry(0, 1.0)]),
        (2.0, vec![entry(0, 1.0), entry(2, 0.75)]),
    ]
}

#[test]
fn a_document_whose_score_cancels_to_zero_is_returned_once() {
    let term_postings = cancelling_term_postings();

    let dense = linear_scan_dense(10, 2, &term_postings);

    let mut ids: Vec<u64> = dense.iter().map(|r| r.doc_id).collect();
    ids.sort_unstable();
    let distinct = {
        let mut d = ids.clone();
        d.dedup();
        d
    };
    assert_eq!(
        ids, distinct,
        "linear_scan_dense returned a duplicate document: {ids:?}"
    );
}

#[test]
fn both_accumulators_agree_on_a_cancelling_query() {
    let term_postings = cancelling_term_postings();

    let dense = linear_scan_dense(10, 2, &term_postings);
    let hashmap = linear_scan_hashmap(10, &term_postings);

    assert_eq!(
        as_pairs(&dense),
        as_pairs(&hashmap),
        "dense and hash map accumulators must be interchangeable"
    );
}

/// The duplicate does not merely repeat a result — it occupies a slot in the
/// top-k heap, evicting a document that genuinely belongs there.
#[test]
fn a_duplicate_must_not_evict_a_genuine_result_from_top_k() {
    let term_postings = cancelling_term_postings();

    // Truth: doc 1 = 5.0, doc 0 = 1.0 - 1.0 + 2.0 = 2.0, doc 2 = 1.5.
    let dense = linear_scan_dense(3, 2, &term_postings);

    let ids: Vec<u64> = dense.iter().map(|r| r.doc_id).collect();
    assert_eq!(
        ids,
        vec![1, 0, 2],
        "top-3 must hold three distinct documents, got {ids:?}"
    );
}

#[test]
fn both_accumulators_agree_on_a_plain_positive_query() {
    let term_postings = vec![
        (1.0, vec![entry(0, 1.0), entry(2, 3.0)]),
        (0.5, vec![entry(1, 4.0), entry(2, 2.0)]),
    ];

    let dense = linear_scan_dense(10, 2, &term_postings);
    let hashmap = linear_scan_hashmap(10, &term_postings);

    assert_eq!(as_pairs(&dense), as_pairs(&hashmap));
}
