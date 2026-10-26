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

// --- A caller-supplied `k` must not size the heap -------------------------
//
// `BinaryHeap::with_capacity(k + 1)` asked the allocator for `k + 1` entries up
// front. The top-k a client sends reaches this module unclamped, so a huge `k`
// aborted the process on allocation failure and `usize::MAX` overflowed the
// `+ 1` (a panic in debug builds). The heap never holds more entries than there
// are scored documents, so the reservation is an optimisation, not a need.

mod unbounded_k {
    use super::super::maxscore_search;
    use super::{entry, linear_scan_dense, linear_scan_hashmap};
    use crate::sparse_index::inverted_index::SparseInvertedIndex;
    use crate::sparse_index::search::sparse_search;
    use crate::sparse_index::types::SparseVector;

    const HUGE_K: [usize; 3] = [1 << 40, usize::MAX - 1, usize::MAX];

    fn two_doc_postings() -> Vec<(f32, Vec<crate::sparse_index::types::PostingEntry>)> {
        vec![(1.0, vec![entry(0, 1.0), entry(1, 2.0)])]
    }

    fn two_doc_index() -> (SparseInvertedIndex, SparseVector) {
        let index = SparseInvertedIndex::new();
        index.insert(0, &SparseVector::new(vec![(1, 1.0)]));
        index.insert(1, &SparseVector::new(vec![(1, 2.0)]));
        (index, SparseVector::new(vec![(1, 1.0)]))
    }

    #[test]
    fn the_dense_accumulator_returns_every_document_for_a_huge_k() {
        for k in HUGE_K {
            let ids: Vec<u64> = linear_scan_dense(k, 1, &two_doc_postings())
                .iter()
                .map(|r| r.doc_id)
                .collect();
            assert_eq!(ids, vec![1, 0], "k = {k}");
        }
    }

    #[test]
    fn the_hash_map_accumulator_returns_every_document_for_a_huge_k() {
        for k in HUGE_K {
            let ids: Vec<u64> = linear_scan_hashmap(k, &two_doc_postings())
                .iter()
                .map(|r| r.doc_id)
                .collect();
            assert_eq!(ids, vec![1, 0], "k = {k}");
        }
    }

    #[test]
    fn maxscore_returns_every_document_for_a_huge_k() {
        let (index, query) = two_doc_index();
        for k in HUGE_K {
            let ids: Vec<u64> = maxscore_search(&index, &query, k)
                .iter()
                .map(|r| r.doc_id)
                .collect();
            assert_eq!(ids, vec![1, 0], "k = {k}");
        }
    }

    #[test]
    fn the_filtered_router_survives_a_k_whose_oversampling_would_overflow() {
        use crate::sparse_index::search::sparse_search_filtered;
        let (index, query) = two_doc_index();
        let keep_all = |_: u64| true;
        for k in HUGE_K {
            let ids: Vec<u64> = sparse_search_filtered(&index, &query, k, Some(&keep_all))
                .iter()
                .map(|r| r.doc_id)
                .collect();
            assert_eq!(ids, vec![1, 0], "k = {k}");
        }
    }

    #[test]
    fn the_router_returns_every_document_for_a_huge_k() {
        let (index, query) = two_doc_index();
        for k in HUGE_K {
            let ids: Vec<u64> = sparse_search(&index, &query, k)
                .iter()
                .map(|r| r.doc_id)
                .collect();
            assert_eq!(ids, vec![1, 0], "k = {k}");
        }
    }
}
