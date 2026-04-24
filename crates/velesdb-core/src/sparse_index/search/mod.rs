//! DAAT `MaxScore` sparse search with linear scan fallback.
//!
//! Provides inner-product ANN search over a [`SparseInvertedIndex`].
//! `MaxScore` partitions query terms into essential/non-essential sets for
//! early termination. A linear scan fallback handles high-coverage queries
//! where `MaxScore` overhead exceeds its benefit.

#![allow(clippy::cast_precision_loss)]

mod scoring;
mod strategy;

#[cfg(test)]
mod bmw_parity_tests;

use super::inverted_index::SparseInvertedIndex;
use super::types::{ScoredDoc, SparseVector};
use strategy::{linear_scan_search, maxscore_search};

/// When total posting list length exceeds this fraction of
/// `doc_count * num_query_terms`, the linear scan fallback is used.
const FULL_SCAN_THRESHOLD: f32 = 0.3;

/// Doc-count threshold below which the linear scan path is preferred
/// unconditionally.
///
/// At this size the dense accumulator (`4 * doc_count` bytes) fits in L2
/// cache (`<= 400 KB` for 100K docs on typical x86-64 CPUs), so a single
/// cache-hot pass over the posting entries beats the cursor / heap /
/// binary-search overhead of `MaxScore` DAAT. Above this threshold the
/// existing coverage-based heuristic chooses between DAAT and linear scan.
///
/// Empirical basis: on 10K SPLADE-like corpus, forcing linear scan dropped
/// `sparse_search top10` latency from ~956 µs to ~60 µs — a 15x speedup
/// that stems from the dense-accumulator path being cache-friendly for
/// moderate corpora. Reassess this constant once a million-doc sparse
/// benchmark is added.
const SMALL_CORPUS_LINEAR_THRESHOLD: u64 = 100_000;

/// Maximum doc ID for which we use a dense accumulator array.
/// Above this threshold we fall back to a hash map.
///
/// Capped at `1_000_000` to bound the worst-case allocation to ~4 MB
/// (`(max_doc_id + 1) * size_of::<f32>() == ~4 MB`). The density check
/// in `linear_scan_search` further restricts this path to compact ID spaces.
const MAX_DENSE_ACCUMULATOR: u64 = 1_000_000;

/// Searches the sparse inverted index for the top-k documents by inner product.
///
/// Routing strategy (in order):
/// 1. Small corpora (`doc_count <= SMALL_CORPUS_LINEAR_THRESHOLD`) always use
///    linear scan — the dense accumulator stays L2-resident and the tight
///    loop beats DAAT overhead.
/// 2. Queries with any negative weight use linear scan (the `MaxScore` upper
///    bound is only valid for non-negative query / document weights — see
///    CRITICAL-1 below).
/// 3. Queries whose coverage (`total_postings / (doc_count * nnz)`) exceeds
///    `FULL_SCAN_THRESHOLD` use linear scan because DAAT pruning would
///    contribute little.
/// 4. Otherwise, `MaxScore` DAAT with early termination.
#[must_use]
pub fn sparse_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<ScoredDoc> {
    if k == 0 || query.is_empty() || index.doc_count() == 0 {
        return Vec::new();
    }

    let doc_count = index.doc_count();

    // CRITICAL-1: MaxScore DAAT computes upper bounds as
    // `query_weight.abs() * max_doc_weight`, which is incorrect when query
    // weights are negative and document weights are also negative (the inner
    // product can be positive but the bound treats it as zero-or-negative
    // contribution). Fall back to linear scan for any query with negative weights.
    let has_negative_weight = query.values.iter().any(|&w| w < 0.0);

    // Small corpus fast path: dense linear scan keeps the entire accumulator
    // in L2 cache and avoids DAAT's cursor/heap overhead.
    if doc_count <= SMALL_CORPUS_LINEAR_THRESHOLD || has_negative_weight {
        return linear_scan_search(index, query, k);
    }

    // Coverage heuristic for large corpora: linear scan still wins when the
    // query terms cover a large slice of the index.
    let mut total_postings: usize = 0;
    for &term_id in &query.indices {
        total_postings += index.posting_count(term_id);
    }
    let coverage_threshold = FULL_SCAN_THRESHOLD * doc_count as f32 * query.nnz() as f32;
    if (total_postings as f32) > coverage_threshold {
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
    use rustc_hash::FxHashMap;

    if k == 0 || query.is_empty() || index.doc_count() == 0 {
        return Vec::new();
    }

    let mut scores: FxHashMap<u64, f32> = FxHashMap::default();
    for (&term_id, &qw) in query.indices.iter().zip(query.values.iter()) {
        let postings = index.get_all_postings(term_id);
        for entry in &postings {
            *scores.entry(entry.doc_id).or_insert(0.0) += qw * entry.weight;
        }
    }

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
    use super::super::inverted_index::SparseInvertedIndex;
    use super::super::types::SparseVector;
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
        index.insert(0, &make_vector(vec![(1, 1.0), (2, 2.0)]));
        index.insert(1, &make_vector(vec![(1, 3.0)]));
        index.insert(2, &make_vector(vec![(2, 1.0), (3, 1.0)]));

        let query = make_vector(vec![(1, 1.0), (2, 1.0)]);
        let results = sparse_search(&index, &query, 2);
        assert_eq!(results.len(), 2);
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
        index.insert(0, &make_vector(vec![(1, 2.0), (2, 3.0)]));
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
        let index = SparseInvertedIndex::new();
        for i in 0..100_u64 {
            index.insert(i, &make_vector(vec![(1, 1.0), (2, 0.5)]));
        }

        let query = make_vector(vec![(1, 1.0), (2, 1.0)]);
        let results = sparse_search(&index, &query, 5);

        assert_eq!(results.len(), 5);
        for r in &results {
            assert!((r.score - 1.5).abs() < 1e-5, "score={}", r.score);
        }
    }

    // --- MaxScore partitioning ---

    #[test]
    fn test_maxscore_5_terms_partitioning() {
        let index = SparseInvertedIndex::new();
        index.insert(
            0,
            &make_vector(vec![(1, 0.1), (2, 0.2), (3, 0.5), (4, 1.0), (5, 2.0)]),
        );
        index.insert(1, &make_vector(vec![(4, 3.0), (5, 4.0)]));
        index.insert(2, &make_vector(vec![(1, 5.0), (2, 3.0)]));

        let query = make_vector(vec![(1, 1.0), (2, 1.0), (3, 1.0), (4, 1.0), (5, 1.0)]);
        let results = sparse_search(&index, &query, 3);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].doc_id, 2); // 8.0
        assert_eq!(results[1].doc_id, 1); // 7.0
        assert_eq!(results[2].doc_id, 0); // 3.8
    }

    // --- Filtered sparse search tests ---

    #[test]
    fn test_sparse_search_filtered_basic() {
        let index = SparseInvertedIndex::new();
        for i in 0..20_u64 {
            index.insert(i, &make_vector(vec![(1, 1.0 + i as f32)]));
        }

        let query = make_vector(vec![(1, 1.0)]);
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

    #[test]
    fn test_maxscore_negative_weights() {
        let index = SparseInvertedIndex::new();
        index.insert(0, &make_vector(vec![(1, 2.0), (2, -1.0), (3, 0.5)]));
        index.insert(1, &make_vector(vec![(1, 1.0), (2, 3.0)]));
        index.insert(2, &make_vector(vec![(2, -2.0), (3, 4.0)]));
        index.insert(3, &make_vector(vec![(1, -0.5), (3, 1.0)]));

        let query = make_vector(vec![(1, 1.0), (2, -1.0), (3, 1.0)]);

        let bf = brute_force_search(&index, &query, 4);
        let ms = sparse_search(&index, &query, 4);

        let bf_ids: Vec<u64> = bf.iter().map(|r| r.doc_id).collect();
        let ms_ids: Vec<u64> = ms.iter().map(|r| r.doc_id).collect();
        assert_eq!(
            bf_ids, ms_ids,
            "MaxScore must match brute-force with mixed-sign weights"
        );

        assert_eq!(ms[0].doc_id, 2, "doc 2 should score highest (6.0)");
        assert!((ms[0].score - 6.0).abs() < 1e-5, "score={}", ms[0].score);
    }
}
