use super::super::inverted_index::SparseInvertedIndex;
use super::super::types::SparseVector;
use super::*;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

fn make_vector(pairs: Vec<(u32, f32)>) -> SparseVector {
    SparseVector::new(pairs)
}

// --- Helper: generate SPLADE-like sparse vectors ---
fn generate_splade_corpus(n: usize, seed: u64) -> Vec<SparseVector> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let nnz = rng.random_range(50..=200);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used = std::collections::HashSet::new();
            while pairs.len() < nnz {
                let term_id = rng.random_range(0..30_000_u32);
                if used.insert(term_id) {
                    let weight = rng.random_range(0.01_f32..2.0);
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
            let nnz = rng.random_range(20..=60);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used = std::collections::HashSet::new();
            while pairs.len() < nnz {
                let term_id = rng.random_range(0..30_000_u32);
                if used.insert(term_id) {
                    let weight = rng.random_range(0.01_f32..2.0);
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
    let filter = |id: u64| id.is_multiple_of(2);
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
