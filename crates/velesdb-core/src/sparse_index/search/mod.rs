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
/// Tied to [`MAX_DENSE_ACCUMULATOR`]: the whole regime where the linear
/// scan gets its dense accumulator is routed to it (#2177). The old value
/// of `100_000` created a cliff at the boundary — measured with the
/// `sparse_posting_scale` bench (both query shapes, two passes each,
/// checksums stable): crossing 100k docs cost **43×** per query on
/// uniform queries and **14×** on skewed SPLADE-shaped ones, and the
/// ratio stayed flat (~13×) in a crossover sweep up to 400k docs — there
/// is no crossover inside the dense regime. The mechanism (confirmed by
/// an operation-count probe on #2177): this `MaxScore` implementation
/// scores the same candidate union a linear scan visits — it prunes no
/// candidates, but pays cursor bookkeeping and a per-candidate min-scan
/// over the essential lists, ~100–280 elementary ops per posting where
/// the linear scan pays ~1.
///
/// Beyond this threshold (non-compact doc-id spaces above it, too) the
/// coverage heuristic below still chooses between DAAT and linear scan;
/// whether `MaxScore` earns its keep past 1M docs is unmeasured — see
/// #2177 for the retirement question.
const SMALL_CORPUS_LINEAR_THRESHOLD: u64 = MAX_DENSE_ACCUMULATOR;

/// Maximum doc ID for which we use a dense accumulator array.
/// Above this threshold we fall back to a hash map.
///
/// Capped at `1_000_000` to bound the worst-case allocation to ~5 MB — a
/// `f32` score plus a `bool` membership flag per slot
/// (`(max_doc_id + 1) * (size_of::<f32>() + size_of::<bool>())`). The density
/// check in `linear_scan_search` further restricts this path to compact ID
/// spaces.
const MAX_DENSE_ACCUMULATOR: u64 = 1_000_000;

/// Searches the sparse inverted index for the top-k documents by inner product.
///
/// Routing strategy (in order):
/// 1. Corpora in the dense-accumulator regime
///    (`doc_count <= SMALL_CORPUS_LINEAR_THRESHOLD`) always use linear
///    scan — measured 14–43× faster than `MaxScore` at the former 100k
///    boundary and still ~13× ahead at 400k (#2177).
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

    // Dense-regime fast path: the linear scan's accumulator pays ~1
    // elementary op per posting where this MaxScore pays ~100-280 (#2177).
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
    all_docs.sort_unstable_by(|a, b| b.cmp(a)); // descending
    all_docs.truncate(k);
    all_docs
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
