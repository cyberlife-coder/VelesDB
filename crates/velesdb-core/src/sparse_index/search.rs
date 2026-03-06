//! DAAT `MaxScore` sparse search with linear scan fallback.
//!
//! Provides inner-product ANN search over a [`SparseInvertedIndex`].
//! `MaxScore` partitions query terms into essential/non-essential sets for
//! early termination. A linear scan fallback handles high-coverage queries
//! where `MaxScore` overhead exceeds its benefit.

#![allow(clippy::cast_precision_loss)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rustc_hash::FxHashMap;

use super::inverted_index::SparseInvertedIndex;
use super::types::{PostingEntry, ScoredDoc, SparseVector};

/// When total posting list length exceeds this fraction of
/// `doc_count * num_query_terms`, the linear scan fallback is used.
const FULL_SCAN_THRESHOLD: f32 = 0.3;

/// Maximum doc ID for which we use a dense accumulator array.
/// Above this threshold we fall back to a hash map.
const MAX_DENSE_ACCUMULATOR: u64 = 10_000_000;

/// Searches the sparse inverted index for the top-k documents by inner product.
///
/// Automatically selects between `MaxScore` DAAT and linear scan based on a
/// coverage heuristic.
#[must_use]
pub fn sparse_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<ScoredDoc> {
    if k == 0 || query.is_empty() || index.doc_count() == 0 {
        return Vec::new();
    }

    // Decide search strategy based on coverage heuristic.
    // Use `posting_count` (no Vec allocation) rather than materialising
    // `get_all_postings` and immediately discarding the result.
    let doc_count = index.doc_count();
    let mut total_postings: usize = 0;
    for &term_id in &query.indices {
        total_postings += index.posting_count(term_id);
    }

    let threshold = FULL_SCAN_THRESHOLD * doc_count as f32 * query.nnz() as f32;
    let use_linear = (total_postings as f32) > threshold;

    if use_linear {
        linear_scan_search(index, query, k)
    } else {
        maxscore_search(index, query, k)
    }
}

/// Searches the sparse inverted index with an optional post-filter.
///
/// If `filter` is `None`, delegates to [`sparse_search`]. Otherwise,
/// retrieves `k * 4` candidates, applies the filter, and retries with
/// `k * 8` if fewer than `k` results survive. Returns the top-k filtered
/// results.
#[must_use]
pub fn sparse_search_filtered(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
    filter: Option<&dyn Fn(u64) -> bool>,
) -> Vec<ScoredDoc> {
    let Some(filter) = filter else {
        return sparse_search(index, query, k);
    };

    // First pass: 4x oversampling
    let candidates = sparse_search(index, query, k.saturating_mul(4).max(k + 10));
    let mut filtered: Vec<ScoredDoc> = candidates
        .into_iter()
        .filter(|doc| filter(doc.doc_id))
        .collect();

    if filtered.len() >= k {
        filtered.truncate(k);
        return filtered;
    }

    // Second pass: 8x oversampling
    let candidates = sparse_search(index, query, k.saturating_mul(8).max(k + 20));
    filtered = candidates
        .into_iter()
        .filter(|doc| filter(doc.doc_id))
        .collect();
    filtered.truncate(k);
    filtered
}

/// Collects posting lists for each query term, along with query weight and
/// the global max document weight for that term.
struct TermPostings {
    query_weight: f32,
    max_doc_weight: f32,
    postings: Vec<PostingEntry>,
}

/// `MaxScore` DAAT search over the inverted index.
fn maxscore_search(index: &SparseInvertedIndex, query: &SparseVector, k: usize) -> Vec<ScoredDoc> {
    // Collect posting lists for each query term
    let mut term_data: Vec<TermPostings> = Vec::with_capacity(query.nnz());
    for (&term_id, &qw) in query.indices.iter().zip(query.values.iter()) {
        let postings = index.get_all_postings(term_id);
        if postings.is_empty() {
            continue;
        }
        let max_dw = index.get_global_max_weight(term_id);
        term_data.push(TermPostings {
            query_weight: qw,
            max_doc_weight: max_dw,
            postings,
        });
    }

    if term_data.is_empty() {
        return Vec::new();
    }

    // Sort by max_contribution ascending (least contributing first)
    term_data.sort_by(|a, b| {
        let ca = a.query_weight.abs() * a.max_doc_weight;
        let cb = b.query_weight.abs() * b.max_doc_weight;
        ca.total_cmp(&cb)
    });

    // Compute prefix sums of max contributions (upper_bound[i] = sum of 0..=i)
    let n_terms = term_data.len();
    let mut upper_bound = vec![0.0_f32; n_terms];
    upper_bound[0] = term_data[0].query_weight.abs() * term_data[0].max_doc_weight;
    for i in 1..n_terms {
        upper_bound[i] =
            upper_bound[i - 1] + term_data[i].query_weight.abs() * term_data[i].max_doc_weight;
    }

    // Min-heap of top-k results
    let mut heap: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    let mut threshold: f32 = 0.0;

    // Find initial split: smallest i where upper_bound[i] >= threshold
    let mut split = find_split(&upper_bound, threshold);

    // Merge-traverse all essential posting lists (split..n_terms) in doc_id order
    // Use cursor-based merge
    let mut cursors: Vec<usize> = vec![0; n_terms];

    loop {
        // Find the smallest doc_id among essential posting lists
        let mut min_doc_id: Option<u64> = None;
        for i in split..n_terms {
            if cursors[i] < term_data[i].postings.len() {
                let did = term_data[i].postings[cursors[i]].doc_id;
                match min_doc_id {
                    None => min_doc_id = Some(did),
                    Some(m) if did < m => min_doc_id = Some(did),
                    _ => {}
                }
            }
        }

        let Some(doc_id) = min_doc_id else {
            break; // All essential lists exhausted
        };

        // Compute score from essential terms
        let mut score = 0.0_f32;
        for i in split..n_terms {
            if cursors[i] < term_data[i].postings.len()
                && term_data[i].postings[cursors[i]].doc_id == doc_id
            {
                score += term_data[i].query_weight * term_data[i].postings[cursors[i]].weight;
                cursors[i] += 1;
            }
        }

        // Add contributions from non-essential terms (binary search)
        for td in &term_data[..split] {
            if let Ok(pos) = td.postings.binary_search_by_key(&doc_id, |e| e.doc_id) {
                score += td.query_weight * td.postings[pos].weight;
            }
        }

        // Push to heap if score exceeds threshold (or heap not full)
        if heap.len() < k || score > threshold {
            heap.push(Reverse(ScoredDoc { score, doc_id }));
            if heap.len() > k {
                heap.pop();
            }
            if heap.len() == k {
                threshold = heap.peek().map_or(0.0, |Reverse(s)| s.score);
                // Re-evaluate split
                split = find_split(&upper_bound, threshold);
            }
        }
    }

    extract_sorted_results(heap)
}

/// Finds the split index: smallest i where `upper_bound[i] >= threshold`.
/// Returns `upper_bound.len()` if no such index exists (all terms are non-essential).
fn find_split(upper_bound: &[f32], threshold: f32) -> usize {
    for (i, &ub) in upper_bound.iter().enumerate() {
        if ub >= threshold {
            return i;
        }
    }
    upper_bound.len()
}

/// Linear scan fallback for high-coverage queries.
fn linear_scan_search(
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

    // Choose accumulator strategy based on max_doc_id
    if max_doc_id > MAX_DENSE_ACCUMULATOR {
        linear_scan_hashmap(k, &term_postings)
    } else {
        linear_scan_dense(k, max_doc_id, &term_postings)
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

    // Extract top-k from touched doc_ids
    let mut heap: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    for &doc_id in &touched {
        #[allow(clippy::cast_possible_truncation)]
        let score = scores[doc_id as usize];
        if heap.len() < k {
            heap.push(Reverse(ScoredDoc { score, doc_id }));
        } else if let Some(Reverse(min)) = heap.peek() {
            if score > min.score {
                heap.pop();
                heap.push(Reverse(ScoredDoc { score, doc_id }));
            }
        }
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
        if heap.len() < k {
            heap.push(Reverse(ScoredDoc { score, doc_id }));
        } else if let Some(Reverse(min)) = heap.peek() {
            if score > min.score {
                heap.pop();
                heap.push(Reverse(ScoredDoc { score, doc_id }));
            }
        }
    }

    extract_sorted_results(heap)
}

/// Extract results from a min-heap and sort descending by score.
fn extract_sorted_results(heap: BinaryHeap<Reverse<ScoredDoc>>) -> Vec<ScoredDoc> {
    let mut results: Vec<ScoredDoc> = heap.into_iter().map(|Reverse(s)| s).collect();
    results.sort_by(|a, b| b.cmp(a)); // descending
    results
}

/// Brute-force inner product search for testing correctness.
///
/// Computes exact inner product for every document by iterating all
/// terms in the index.
#[cfg(test)]
pub(crate) fn brute_force_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<ScoredDoc> {
    if k == 0 || query.is_empty() || index.doc_count() == 0 {
        return Vec::new();
    }

    // Accumulate scores for all documents
    let mut scores: FxHashMap<u64, f32> = FxHashMap::default();

    for (&term_id, &qw) in query.indices.iter().zip(query.values.iter()) {
        let postings = index.get_all_postings(term_id);
        for entry in &postings {
            *scores.entry(entry.doc_id).or_insert(0.0) += qw * entry.weight;
        }
    }

    // Sort all scored docs and take top-k
    let mut all_docs: Vec<ScoredDoc> = scores
        .into_iter()
        .map(|(doc_id, score)| ScoredDoc { score, doc_id })
        .collect();
    all_docs.sort_by(|a, b| b.cmp(a)); // descending
    all_docs.truncate(k);
    all_docs
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    fn make_vector(pairs: Vec<(u32, f32)>) -> SparseVector {
        SparseVector::new(pairs)
    }

    // --- Helper: generate SPLADE-like sparse vectors ---
    fn generate_splade_corpus(n: usize, seed: u64) -> Vec<SparseVector> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..n)
            .map(|_| {
                let nnz = rng.gen_range(50..=200);
                let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
                let mut used = std::collections::HashSet::new();
                while pairs.len() < nnz {
                    let term_id = rng.gen_range(0..30_000_u32);
                    if used.insert(term_id) {
                        let weight = rng.gen_range(0.01_f32..2.0);
                        pairs.push((term_id, weight));
                    }
                }
                SparseVector::new(pairs)
            })
            .collect()
    }

    fn generate_queries(n: usize, seed: u64) -> Vec<SparseVector> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..n)
            .map(|_| {
                let nnz = rng.gen_range(20..=60);
                let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
                let mut used = std::collections::HashSet::new();
                while pairs.len() < nnz {
                    let term_id = rng.gen_range(0..30_000_u32);
                    if used.insert(term_id) {
                        let weight = rng.gen_range(0.01_f32..2.0);
                        pairs.push((term_id, weight));
                    }
                }
                SparseVector::new(pairs)
            })
            .collect()
    }

    // --- Basic tests ---

    #[test]
    fn test_sparse_search_basic_3_docs() {
        let index = SparseInvertedIndex::new();
        // doc 0: terms 1=1.0, 2=2.0
        index.insert(0, &make_vector(vec![(1, 1.0), (2, 2.0)]));
        // doc 1: terms 1=3.0
        index.insert(1, &make_vector(vec![(1, 3.0)]));
        // doc 2: terms 2=1.0, 3=1.0
        index.insert(2, &make_vector(vec![(2, 1.0), (3, 1.0)]));

        // query: terms 1=1.0, 2=1.0
        // doc 0: 1*1 + 2*1 = 3.0
        // doc 1: 3*1 = 3.0
        // doc 2: 1*1 = 1.0
        let query = make_vector(vec![(1, 1.0), (2, 1.0)]);
        let results = sparse_search(&index, &query, 2);
        assert_eq!(results.len(), 2);
        // Top-2 should be docs 0 and 1 (both score 3.0), doc 2 score 1.0
        let ids: Vec<u64> = results.iter().map(|r| r.doc_id).collect();
        assert!(ids.contains(&0));
        assert!(ids.contains(&1));
    }

    #[test]
    fn test_sparse_search_k_greater_than_docs() {
        let index = SparseInvertedIndex::new();
        index.insert(0, &make_vector(vec![(1, 1.0)]));
        index.insert(1, &make_vector(vec![(1, 2.0)]));

        let query = make_vector(vec![(1, 1.0)]);
        let results = sparse_search(&index, &query, 10);
        assert_eq!(results.len(), 2);
        // Higher score first
        assert_eq!(results[0].doc_id, 1);
        assert_eq!(results[1].doc_id, 0);
    }

    #[test]
    fn test_sparse_search_empty_index() {
        let index = SparseInvertedIndex::new();
        let query = make_vector(vec![(1, 1.0)]);
        let results = sparse_search(&index, &query, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_sparse_search_empty_query() {
        let index = SparseInvertedIndex::new();
        index.insert(0, &make_vector(vec![(1, 1.0)]));
        let query = make_vector(vec![]);
        let results = sparse_search(&index, &query, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_sparse_search_k_zero() {
        let index = SparseInvertedIndex::new();
        index.insert(0, &make_vector(vec![(1, 1.0)]));
        let query = make_vector(vec![(1, 1.0)]);
        let results = sparse_search(&index, &query, 0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_sparse_search_raw_inner_product() {
        let index = SparseInvertedIndex::new();
        // doc 0: term 1=2.0, term 2=3.0
        index.insert(0, &make_vector(vec![(1, 2.0), (2, 3.0)]));
        // query: term 1=1.5, term 2=0.5
        // expected: 2.0*1.5 + 3.0*0.5 = 4.5
        let query = make_vector(vec![(1, 1.5), (2, 0.5)]);
        let results = sparse_search(&index, &query, 1);
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 4.5).abs() < 1e-5);
    }

    // --- Brute-force vs MaxScore correctness ---

    #[test]
    fn test_maxscore_matches_brute_force_1k_corpus() {
        let corpus = generate_splade_corpus(1000, 42);
        let queries = generate_queries(50, 123);

        let index = SparseInvertedIndex::new();
        for (i, vec) in corpus.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            index.insert(i as u64, vec);
        }

        for (qi, query) in queries.iter().enumerate() {
            let bf_results = brute_force_search(&index, query, 10);
            let ms_results = sparse_search(&index, query, 10);

            let bf_ids: Vec<u64> = bf_results.iter().map(|r| r.doc_id).collect();
            let ms_ids: Vec<u64> = ms_results.iter().map(|r| r.doc_id).collect();
            assert_eq!(
                bf_ids, ms_ids,
                "Query {qi}: MaxScore result IDs differ from brute-force"
            );
        }
    }

    // --- Linear scan fallback ---

    #[test]
    fn test_linear_scan_fallback_correctness() {
        // Create index where query terms cover >30% of docs
        // to force linear scan. Use terms shared by all docs.
        let index = SparseInvertedIndex::new();
        for i in 0..100_u64 {
            // All docs share terms 1 and 2
            index.insert(i, &make_vector(vec![(1, 1.0), (2, 0.5)]));
        }

        // Query on shared terms => total_postings = 200, doc_count=100, query_nnz=2
        // threshold = 0.3 * 100 * 2 = 60. total_postings=200 > 60 => linear scan
        let query = make_vector(vec![(1, 1.0), (2, 1.0)]);
        let results = sparse_search(&index, &query, 5);

        // All docs have same score: 1*1 + 0.5*1 = 1.5
        assert_eq!(results.len(), 5);
        for r in &results {
            assert!((r.score - 1.5).abs() < 1e-5, "score={}", r.score);
        }
    }

    // --- MaxScore partitioning ---

    #[test]
    fn test_maxscore_5_terms_partitioning() {
        // 5-term query with varying contributions, verify correct results
        let index = SparseInvertedIndex::new();
        // doc 0: all 5 terms with different weights
        index.insert(
            0,
            &make_vector(vec![(1, 0.1), (2, 0.2), (3, 0.5), (4, 1.0), (5, 2.0)]),
        );
        // doc 1: only high-value terms
        index.insert(1, &make_vector(vec![(4, 3.0), (5, 4.0)]));
        // doc 2: only low-value terms
        index.insert(2, &make_vector(vec![(1, 5.0), (2, 3.0)]));

        let query = make_vector(vec![(1, 1.0), (2, 1.0), (3, 1.0), (4, 1.0), (5, 1.0)]);
        let results = sparse_search(&index, &query, 3);

        // doc 0: 0.1+0.2+0.5+1.0+2.0 = 3.8
        // doc 1: 3.0+4.0 = 7.0
        // doc 2: 5.0+3.0 = 8.0
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].doc_id, 2); // 8.0
        assert_eq!(results[1].doc_id, 1); // 7.0
        assert_eq!(results[2].doc_id, 0); // 3.8
    }

    // --- Negative weight correctness ---

    // --- Filtered sparse search tests ---

    #[test]
    fn test_sparse_search_filtered_basic() {
        let index = SparseInvertedIndex::new();
        for i in 0..20_u64 {
            index.insert(i, &make_vector(vec![(1, 1.0 + i as f32)]));
        }

        let query = make_vector(vec![(1, 1.0)]);

        // Filter: only even doc IDs
        let filter = |id: u64| id % 2 == 0;
        let results = sparse_search_filtered(&index, &query, 5, Some(&filter));

        assert_eq!(results.len(), 5);
        for r in &results {
            assert_eq!(r.doc_id % 2, 0, "doc {} should be even", r.doc_id);
        }
    }

    #[test]
    fn test_sparse_search_filtered_none() {
        let index = SparseInvertedIndex::new();
        for i in 0..10_u64 {
            index.insert(i, &make_vector(vec![(1, 1.0 + i as f32)]));
        }

        let query = make_vector(vec![(1, 1.0)]);

        let unfiltered = sparse_search(&index, &query, 5);
        let filtered_none = sparse_search_filtered(&index, &query, 5, None);

        assert_eq!(unfiltered.len(), filtered_none.len());
        for (a, b) in unfiltered.iter().zip(filtered_none.iter()) {
            assert_eq!(a.doc_id, b.doc_id);
            assert!((a.score - b.score).abs() < 1e-5);
        }
    }

    /// Verify that `MaxScore` produces the same top-k as brute-force when
    /// document vectors contain negative weights (e.g. SPLADE with negatives).
    #[test]
    fn test_maxscore_negative_weights() {
        let index = SparseInvertedIndex::new();
        // doc 0: mixed positive and negative weights
        index.insert(0, &make_vector(vec![(1, 2.0), (2, -1.0), (3, 0.5)]));
        // doc 1: all positive
        index.insert(1, &make_vector(vec![(1, 1.0), (2, 3.0)]));
        // doc 2: benefits from negative query term
        index.insert(2, &make_vector(vec![(2, -2.0), (3, 4.0)]));
        // doc 3: single negative + positive
        index.insert(3, &make_vector(vec![(1, -0.5), (3, 1.0)]));

        // query: term 1 positive, term 2 negative, term 3 positive
        let query = make_vector(vec![(1, 1.0), (2, -1.0), (3, 1.0)]);

        // Expected inner products:
        // doc 0: 2*1 + (-1)*(-1) + 0.5*1 = 3.5
        // doc 1: 1*1 + 3*(-1) = -2.0
        // doc 2: (-2)*(-1) + 4*1 = 6.0
        // doc 3: (-0.5)*1 + 1*1 = 0.5

        let bf = brute_force_search(&index, &query, 4);
        let ms = sparse_search(&index, &query, 4);

        let bf_ids: Vec<u64> = bf.iter().map(|r| r.doc_id).collect();
        let ms_ids: Vec<u64> = ms.iter().map(|r| r.doc_id).collect();
        assert_eq!(
            bf_ids, ms_ids,
            "MaxScore must match brute-force with mixed-sign weights"
        );

        // Spot-check top result
        assert_eq!(ms[0].doc_id, 2, "doc 2 should score highest (6.0)");
        assert!((ms[0].score - 6.0).abs() < 1e-5, "score={}", ms[0].score);
    }
}
