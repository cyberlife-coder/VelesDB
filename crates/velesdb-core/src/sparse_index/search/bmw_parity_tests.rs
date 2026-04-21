//! Parity tests for the sparse search hot path.
//!
//! These tests form the TDD contract for Phase 4.2 (Block-Max WAND + allocation
//! elimination, issue #378). They MUST pass both on `develop` (pre-refactor) and
//! after the hot-path refactor lands — any divergence during development is a
//! regression that blocks further work.
//!
//! Three parity surfaces are covered:
//! 1. Current `sparse_search` result IDs match [`brute_force_search`] exactly on
//!    positive-weight SPLADE-like corpora at 10K corpus / 100 queries.
//! 2. Scores agree to f32 precision (`< 1e-4` relative).
//! 3. Negative-weight queries still produce brute-force-equivalent results via
//!    the linear-scan fallback route.
//!
//! Also asserts correctness across edge cases that the production benchmark
//! (`benches/sparse_benchmark.rs`) does not exercise: multi-segment corpora,
//! `k > doc_count`, empty posting lists, and mixed-sign queries.

use std::collections::HashSet;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::super::inverted_index::{SparseInvertedIndex, FREEZE_THRESHOLD};
use super::super::types::{ScoredDoc, SparseVector};
use super::{brute_force_search, sparse_search};

const VOCAB_SIZE: u32 = 30_000;

fn gen_positive_corpus(n: usize, seed: u64) -> Vec<SparseVector> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let nnz = rng.gen_range(50..=200);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used: HashSet<u32> = HashSet::new();
            while pairs.len() < nnz {
                let term_id = rng.gen_range(0..VOCAB_SIZE);
                if used.insert(term_id) {
                    let weight = rng.gen_range(0.01_f32..2.0);
                    pairs.push((term_id, weight));
                }
            }
            SparseVector::new(pairs)
        })
        .collect()
}

fn gen_queries(n: usize, seed: u64) -> Vec<SparseVector> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let nnz = rng.gen_range(20..=60);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used: HashSet<u32> = HashSet::new();
            while pairs.len() < nnz {
                let term_id = rng.gen_range(0..VOCAB_SIZE);
                if used.insert(term_id) {
                    let weight = rng.gen_range(0.01_f32..2.0);
                    pairs.push((term_id, weight));
                }
            }
            SparseVector::new(pairs)
        })
        .collect()
}

fn gen_mixed_sign_queries(n: usize, seed: u64) -> Vec<SparseVector> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let nnz = rng.gen_range(10..=30);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used: HashSet<u32> = HashSet::new();
            while pairs.len() < nnz {
                let term_id = rng.gen_range(0..VOCAB_SIZE);
                if used.insert(term_id) {
                    let sign = if rng.gen_bool(0.3) { -1.0 } else { 1.0 };
                    let weight = sign * rng.gen_range(0.1_f32..2.0);
                    pairs.push((term_id, weight));
                }
            }
            SparseVector::new(pairs)
        })
        .collect()
}

fn build_index(corpus: &[SparseVector]) -> SparseInvertedIndex {
    let index = SparseInvertedIndex::new();
    for (i, vec) in corpus.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        index.insert(i as u64, vec);
    }
    index
}

fn doc_ids(results: &[ScoredDoc]) -> Vec<u64> {
    results.iter().map(|r| r.doc_id).collect()
}

fn assert_scores_close(expected: &[ScoredDoc], actual: &[ScoredDoc], query_label: &str) {
    assert_eq!(
        expected.len(),
        actual.len(),
        "{query_label}: length mismatch ({} vs {})",
        expected.len(),
        actual.len()
    );
    for (i, (a, b)) in expected.iter().zip(actual.iter()).enumerate() {
        assert_eq!(
            a.doc_id, b.doc_id,
            "{query_label} rank {i}: doc_id differs ({} vs {})",
            a.doc_id, b.doc_id
        );
        let denom = a.score.abs().max(b.score.abs()).max(1e-6);
        let rel_err = (a.score - b.score).abs() / denom;
        assert!(
            rel_err < 1e-4,
            "{query_label} rank {i}: score relative error {rel_err} exceeds 1e-4 ({} vs {})",
            a.score,
            b.score
        );
    }
}

// ---------- NOMINAL: large-corpus parity ----------

#[test]
fn test_sparse_search_matches_brute_force_10k_corpus_k10() {
    let corpus = gen_positive_corpus(10_000, 42);
    let queries = gen_queries(100, 123);
    let index = build_index(&corpus);

    for (qi, query) in queries.iter().enumerate() {
        let bf = brute_force_search(&index, query, 10);
        let ms = sparse_search(&index, query, 10);
        assert_eq!(
            doc_ids(&bf),
            doc_ids(&ms),
            "Query {qi}: sparse_search IDs diverge from brute-force"
        );
    }
}

#[test]
fn test_sparse_search_matches_brute_force_10k_corpus_k100() {
    let corpus = gen_positive_corpus(10_000, 7);
    let queries = gen_queries(50, 9);
    let index = build_index(&corpus);

    for (qi, query) in queries.iter().enumerate() {
        let bf = brute_force_search(&index, query, 100);
        let ms = sparse_search(&index, query, 100);
        assert_eq!(
            doc_ids(&bf),
            doc_ids(&ms),
            "Query {qi}: sparse_search top-100 IDs diverge from brute-force"
        );
    }
}

#[test]
fn test_sparse_search_scores_match_brute_force_10k_corpus() {
    let corpus = gen_positive_corpus(10_000, 101);
    let queries = gen_queries(30, 202);
    let index = build_index(&corpus);

    for (qi, query) in queries.iter().enumerate() {
        let bf = brute_force_search(&index, query, 10);
        let ms = sparse_search(&index, query, 10);
        assert_scores_close(&bf, &ms, &format!("query {qi}"));
    }
}

// ---------- MULTI-SEGMENT: forces both mutable and frozen segments ----------

#[test]
fn test_sparse_search_matches_brute_force_across_multi_segments() {
    let n = FREEZE_THRESHOLD + 2_500;
    let corpus = gen_positive_corpus(n, 55);
    let queries = gen_queries(50, 66);
    let index = build_index(&corpus);

    for (qi, query) in queries.iter().enumerate() {
        let bf = brute_force_search(&index, query, 10);
        let ms = sparse_search(&index, query, 10);
        assert_eq!(
            doc_ids(&bf),
            doc_ids(&ms),
            "Query {qi}: multi-segment search IDs diverge from brute-force"
        );
    }
}

// ---------- NEGATIVE-WEIGHT: must route through linear scan fallback ----------

#[test]
fn test_sparse_search_negative_weight_queries_match_brute_force() {
    let corpus = gen_positive_corpus(5_000, 33);
    let queries = gen_mixed_sign_queries(40, 44);
    let index = build_index(&corpus);

    for (qi, query) in queries.iter().enumerate() {
        let bf = brute_force_search(&index, query, 10);
        let ms = sparse_search(&index, query, 10);
        assert_eq!(
            doc_ids(&bf),
            doc_ids(&ms),
            "Query {qi} (mixed-sign): fallback path IDs diverge from brute-force"
        );
    }
}

// ---------- EDGE CASES ----------

#[test]
fn test_sparse_search_single_doc_corpus() {
    let corpus = gen_positive_corpus(1, 1);
    let queries = gen_queries(5, 2);
    let index = build_index(&corpus);

    for query in &queries {
        let ms = sparse_search(&index, query, 10);
        assert!(
            ms.len() <= 1,
            "single-doc corpus should return at most 1 result"
        );
    }
}

#[test]
fn test_sparse_search_k_greater_than_doc_count() {
    let corpus = gen_positive_corpus(200, 17);
    let query = &gen_queries(1, 18)[0];
    let index = build_index(&corpus);

    let ms = sparse_search(&index, query, 1_000);
    let bf = brute_force_search(&index, query, 1_000);
    assert_eq!(
        doc_ids(&bf),
        doc_ids(&ms),
        "k > doc_count should still yield brute-force-identical result"
    );
    assert!(ms.len() <= 200, "result count cannot exceed corpus size");
}

#[test]
fn test_sparse_search_term_absent_from_corpus_returns_empty_or_partial() {
    let index = SparseInvertedIndex::new();
    index.insert(0, &SparseVector::new(vec![(100, 1.0)]));
    index.insert(1, &SparseVector::new(vec![(200, 2.0)]));

    // Query a term nobody has — must return empty, never panic
    let unknown = SparseVector::new(vec![(99_999, 1.0)]);
    let result = sparse_search(&index, &unknown, 5);
    assert!(result.is_empty(), "unknown-term query must return empty");

    // Query with mixed known/unknown terms — must return partial results
    let mixed = SparseVector::new(vec![(100, 1.0), (99_999, 1.0)]);
    let result = sparse_search(&index, &mixed, 5);
    assert_eq!(result.len(), 1, "only doc 0 matches term 100");
    assert_eq!(result[0].doc_id, 0);
}

#[test]
fn test_sparse_search_all_zero_query_returns_empty() {
    let corpus = gen_positive_corpus(100, 88);
    let index = build_index(&corpus);

    // SparseVector::new filters zero weights, so an all-zero input yields empty
    let empty_query = SparseVector::new(vec![(1, 0.0), (2, 0.0)]);
    let result = sparse_search(&index, &empty_query, 10);
    assert!(result.is_empty(), "empty query must yield empty results");
}

// ---------- REGRESSION: upsert across segment freeze must not double-count ----------
//
// Before the dedup fix in `k_way_merge`, re-inserting a doc after its
// frozen copy was sealed produced two posting entries for the same
// `doc_id`. `linear_scan_search` and `brute_force_search` both accumulate
// `qw * weight` per entry into a hash map, so the stale frozen weight was
// silently added to the fresh mutable weight — the returned score could
// exceed the true inner product by the size of the frozen weight.

#[test]
fn test_sparse_search_upsert_across_segments_uses_latest_weight() {
    let index = SparseInvertedIndex::new();
    // Fill mutable up to FREEZE_THRESHOLD — insertion #FREEZE_THRESHOLD
    // triggers the freeze internally, so every doc ends up in the frozen
    // segment with baseline weight 0.1.
    for i in 0..FREEZE_THRESHOLD {
        #[allow(clippy::cast_possible_truncation)]
        index.insert(i as u64, &SparseVector::new(vec![(1, 0.1)]));
    }
    // Post-freeze: re-insert doc 0 with a much higher weight. The new
    // entry lands in the fresh (empty) mutable segment.
    index.insert(0, &SparseVector::new(vec![(1, 10.0)]));

    let query = SparseVector::new(vec![(1, 1.0)]);
    let results = sparse_search(&index, &query, 5);

    assert!(!results.is_empty(), "expected at least one result");
    assert_eq!(
        results[0].doc_id, 0,
        "doc 0 should be the top match after the upsert"
    );
    assert!(
        (results[0].score - 10.0).abs() < 1e-5,
        "upsert across segments must replace, not add: expected 10.0, got {}",
        results[0].score
    );
}

#[test]
fn test_sparse_search_deterministic_across_invocations() {
    let corpus = gen_positive_corpus(500, 11);
    let queries = gen_queries(5, 12);
    let index = build_index(&corpus);

    for query in &queries {
        let a = sparse_search(&index, query, 10);
        let b = sparse_search(&index, query, 10);
        assert_eq!(doc_ids(&a), doc_ids(&b), "search must be deterministic");
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x.score - y.score).abs() < f32::EPSILON);
        }
    }
}
