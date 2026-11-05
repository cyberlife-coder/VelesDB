//! Sparse inner-product search: linear scan, dense or hash-map accumulator.
//!
//! Provides inner-product ANN search over a [`SparseInvertedIndex`]. There
//! is exactly one production strategy — `strategy::linear_scan_search`,
//! which picks a dense `f32` accumulator inside `MAX_DENSE_ACCUMULATOR`
//! and a hash map above it.
//!
//! `MaxScore` DAAT used to be the routing alternative for large corpora.
//! The scan measured 14–43× faster than `MaxScore` at the former 100k
//! boundary: crossing 100k docs cost **43×** per query on
//! uniform queries and **14×** on skewed SPLADE-shaped ones, with the
//! ratio stayed flat (~13×) in a crossover sweep up to 400k docs and
//! still ~13× ahead at 400k (#2177). The op-count probe then measured
//! **78×** at 1.1M–1.5M — it scores the same candidate union the scan
//! visits (it prunes nothing) but pays ~100–280 elementary ops per
//! posting where the scan pays ~1. With no crossover anywhere,
//! `maxscore_search` survives only under the `internal-bench` and test
//! seams — kept to be re-measured if a workload ever moves the ratio
//! toward 1×.

#![allow(clippy::cast_precision_loss)]

mod scoring;
mod strategy;

#[cfg(test)]
mod bmw_parity_tests;

use super::inverted_index::SparseInvertedIndex;
use super::types::{ScoredDoc, SparseVector};
use strategy::linear_scan_search;
#[cfg(feature = "internal-bench")]
use strategy::maxscore_search;

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
/// Always the linear scan — the only strategy left after the `MaxScore`
/// retirement (#2177): it handled every regime the router used to dispatch,
/// negative query weights included, and picks its accumulator on the
/// doc-id space itself.
#[must_use]
pub fn sparse_search(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<ScoredDoc> {
    if k == 0 || query.is_empty() || index.doc_count() == 0 {
        return Vec::new();
    }
    linear_scan_search(index, query, k)
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
    let candidates = sparse_search(index, query, k.saturating_mul(4).max(k.saturating_add(10)));
    let mut filtered: Vec<ScoredDoc> = candidates
        .into_iter()
        .filter(|doc| filter(doc.doc_id))
        .collect();

    if filtered.len() >= k {
        filtered.truncate(k);
        return filtered;
    }

    // Second pass: 8x oversampling
    let candidates = sparse_search(index, query, k.saturating_mul(8).max(k.saturating_add(20)));
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

/// [`strategy::linear_scan_search`] for the bench seam (#2177).
///
/// `pub(crate)` and `internal-bench`-only: `internal_bench` re-exports it so a
/// harness can score a corpus both ways at one size. The router remains the
/// only entry point application code has.
#[cfg(feature = "internal-bench")]
#[must_use]
pub(crate) fn linear_scan_search_for_bench(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<ScoredDoc> {
    linear_scan_search(index, query, k)
}

/// [`strategy::maxscore_search`] for the bench seam (#2177).
#[cfg(feature = "internal-bench")]
#[must_use]
pub(crate) fn maxscore_search_for_bench(
    index: &SparseInvertedIndex,
    query: &SparseVector,
    k: usize,
) -> Vec<ScoredDoc> {
    maxscore_search(index, query, k)
}
