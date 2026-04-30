//! Criterion benchmark suite for sparse vector insert and search operations.
//!
//! Measures throughput for:
//! - Sequential and parallel insertion of SPLADE-format sparse vectors
//! - Top-10 and top-100 sparse search on a 10K document corpus
//! - 16-thread concurrent insert + search workload

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use std::collections::HashSet;
use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use velesdb_core::index::sparse::{sparse_search, ScoredDoc, SparseInvertedIndex, SparseVector};
#[cfg(feature = "internal-bench")]
use velesdb_core::internal_bench;

/// Sample size used for the recall validation pass.
const RECALL_SAMPLE_SIZE: usize = 20;

/// Minimum acceptable recall@k of the optimized path vs brute-force ground
/// truth. Below this threshold the bench fails loudly — a speedup that
/// silently degrades retrieval semantics is not a valid speedup.
const RECALL_FLOOR: f32 = 0.95;

/// Brute-force top-k sparse inner-product search used as recall ground truth.
///
/// Computes the exact sparse dot product between `query` and every document
/// in `corpus`, then sorts by score descending and keeps the top-k. This is
/// intentionally O(`N` * `nnz_q` * log `nnz_d`) — only used once per bench
/// setup to validate that the optimized `sparse_search` does not degrade
/// recall.
fn brute_force_top_k(corpus: &[SparseVector], query: &SparseVector, k: usize) -> Vec<ScoredDoc> {
    let q_idx = &query.indices;
    let q_val = &query.values;
    let mut scored: Vec<ScoredDoc> = corpus
        .iter()
        .enumerate()
        .filter_map(|(doc_id, doc)| {
            // Merge-join on sorted indices to compute sparse inner product.
            let mut score = 0.0_f32;
            let (mut qi, mut di) = (0_usize, 0_usize);
            while qi < q_idx.len() && di < doc.indices.len() {
                match q_idx[qi].cmp(&doc.indices[di]) {
                    std::cmp::Ordering::Equal => {
                        score += q_val[qi] * doc.values[di];
                        qi += 1;
                        di += 1;
                    }
                    std::cmp::Ordering::Less => qi += 1,
                    std::cmp::Ordering::Greater => di += 1,
                }
            }
            if score > 0.0 {
                Some(ScoredDoc {
                    doc_id: doc_id as u64,
                    score,
                })
            } else {
                None
            }
        })
        .collect();
    // Sort by score descending, tie-break by doc_id ascending for stability.
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.doc_id.cmp(&b.doc_id))
    });
    scored.truncate(k);
    scored
}

/// Compute recall@k of `actual` vs `expected` — fraction of expected IDs
/// that also appear in `actual`. Score ordering is intentionally ignored
/// (recall measures retrieval quality, not ranking).
fn recall_at_k(actual: &[ScoredDoc], expected: &[ScoredDoc]) -> f32 {
    if expected.is_empty() {
        return 1.0;
    }
    let expected_ids: HashSet<u64> = expected.iter().map(|s| s.doc_id).collect();
    let hits = actual
        .iter()
        .filter(|s| expected_ids.contains(&s.doc_id))
        .count();
    hits as f32 / expected.len() as f32
}

/// Generate a corpus of SPLADE-like sparse vectors.
///
/// Each vector has 50-200 nonzero entries with term IDs in `0..30_000`
/// and weights uniformly sampled from `0.01..2.0`.
fn generate_splade_corpus(n: usize, seed: u64) -> Vec<SparseVector> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let nnz = rng.random_range(50..=200);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used = HashSet::new();
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

/// Build an index pre-loaded with a corpus.
fn build_index(corpus: &[SparseVector]) -> SparseInvertedIndex {
    let index = SparseInvertedIndex::new();
    for (i, vec) in corpus.iter().enumerate() {
        index.insert(i as u64, vec);
    }
    index
}

fn sparse_insert_benchmarks(c: &mut Criterion) {
    let corpus = generate_splade_corpus(10_000, 42);

    let mut group = c.benchmark_group("sparse_insert");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(10));

    // Sequential insert of 10K documents
    group.bench_function("sequential_10k", |b| {
        b.iter(|| {
            let index = SparseInvertedIndex::new();
            for (i, vec) in corpus.iter().enumerate() {
                index.insert(black_box(i as u64), black_box(vec));
            }
            index
        });
    });

    // Parallel insert of 10K documents (4 threads, rayon)
    #[cfg(feature = "persistence")]
    group.bench_function("parallel_10k_rayon_doc_granular", |b| {
        use rayon::prelude::*;
        b.iter(|| {
            let index = Arc::new(SparseInvertedIndex::new());
            corpus.par_iter().enumerate().for_each(|(i, vec)| {
                index.insert(black_box(i as u64), black_box(vec));
            });
            index
        });
    });

    group.bench_function("parallel_10k_manual_4x2500_doc_granular", |b| {
        b.iter(|| {
            let index = Arc::new(SparseInvertedIndex::new());
            let corpus = Arc::new(corpus.clone());
            let mut handles = Vec::with_capacity(4);

            for chunk_id in 0..4_usize {
                let index = Arc::clone(&index);
                let docs = Arc::clone(&corpus);
                handles.push(std::thread::spawn(move || {
                    let start = chunk_id * 2500;
                    let end = start + 2500;
                    for i in start..end {
                        index.insert(i as u64, &docs[i]);
                    }
                }));
            }

            for handle in handles {
                handle.join().expect("thread panicked");
            }

            index
        });
    });

    #[cfg(feature = "internal-bench")]
    group.bench_function("parallel_10k_manual_4x2500_chunked", |b| {
        let docs: Vec<(u64, SparseVector)> = corpus
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, vec)| (i as u64, vec))
            .collect();

        b.iter(|| {
            let index = Arc::new(SparseInvertedIndex::new());
            let docs = Arc::new(docs.clone());
            let mut handles = Vec::with_capacity(4);

            for chunk_id in 0..4_usize {
                let index = Arc::clone(&index);
                let docs = Arc::clone(&docs);
                handles.push(std::thread::spawn(move || {
                    let start = chunk_id * 2500;
                    let end = start + 2500;
                    internal_bench::sparse_insert_batch(&index, &docs[start..end]);
                }));
            }

            for handle in handles {
                handle.join().expect("thread panicked");
            }

            index
        });
    });

    group.finish();
}

/// Corpus is synthetic SPLADE-like (random term IDs + weights); it exercises
/// the same code paths as real SPLADE but makes no claim about production
/// retrieval quality. Production recall on realistic data is verified
/// separately via `cargo test test_recall` (HNSW/dense) and dedicated SIFT1M
/// / MS MARCO harnesses. The recall check below asserts that the optimized
/// `sparse_search` (`MaxScore` DAAT + linear-scan fallback) does not regress
/// against brute-force exact inner product on the SAME synthetic corpus —
/// i.e. it proves the optimization is faithful to the scoring semantics.
fn sparse_search_benchmarks(c: &mut Criterion) {
    let corpus = generate_splade_corpus(10_000, 42);
    let queries = generate_splade_corpus(100, 123);
    let index = build_index(&corpus);

    // --- Recall validation (runs once, before any timed measurement) ---------
    //
    // For each top-k, sample a subset of queries, compute brute-force ground
    // truth and compare to `sparse_search`. Aggregated recall MUST stay above
    // RECALL_FLOOR — otherwise the optimization corrupted retrieval semantics
    // and no speedup claim is admissible. The check deliberately runs outside
    // `b.iter(...)` so criterion's timing is not contaminated.
    for &k in &[10_usize, 100] {
        let mut total_recall = 0.0_f32;
        let mut counted = 0_usize;
        for query in queries.iter().take(RECALL_SAMPLE_SIZE) {
            let expected = brute_force_top_k(&corpus, query, k);
            if expected.is_empty() {
                continue;
            }
            let actual = sparse_search(&index, query, k);
            total_recall += recall_at_k(&actual, &expected);
            counted += 1;
        }
        let recall = if counted == 0 {
            1.0
        } else {
            total_recall / counted as f32
        };
        println!(
            "sparse_search recall@{k} = {recall:.4} (over {counted} queries, floor {RECALL_FLOOR})"
        );
        assert!(
            recall >= RECALL_FLOOR,
            "sparse recall@{k} regressed: {recall:.4} < {RECALL_FLOOR}"
        );
    }

    let mut group = c.benchmark_group("sparse_search");
    group.sample_size(20);
    group.measurement_time(std::time::Duration::from_secs(10));

    // Top-10 search
    group.bench_function("top10_10k_corpus", |b| {
        let mut qi = 0;
        b.iter(|| {
            let query = &queries[qi % queries.len()];
            qi += 1;
            sparse_search(black_box(&index), black_box(query), 10)
        });
    });

    // Top-100 search
    group.bench_function("top100_10k_corpus", |b| {
        let mut qi = 0;
        b.iter(|| {
            let query = &queries[qi % queries.len()];
            qi += 1;
            sparse_search(black_box(&index), black_box(query), 100)
        });
    });

    group.finish();
}

fn sparse_concurrent_benchmarks(c: &mut Criterion) {
    let corpus = generate_splade_corpus(10_000, 42);
    let queries = generate_splade_corpus(100, 123);

    let mut group = c.benchmark_group("sparse_concurrent_insert_search");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(15));

    // 16-thread benchmark: 8 inserting, 8 searching
    group.bench_function("16_threads_8_insert_8_search", |b| {
        b.iter(|| {
            let index = Arc::new(SparseInvertedIndex::new());
            // Pre-load some data so searchers have something to find
            for (i, vec) in corpus.iter().take(1000).enumerate() {
                index.insert(i as u64, vec);
            }

            let mut handles = Vec::with_capacity(16);

            // 8 insert threads
            for thread_id in 0..8_u64 {
                let idx = Arc::clone(&index);
                let docs = corpus.clone();
                handles.push(std::thread::spawn(move || {
                    let start = (thread_id as usize * 1000) + 1000;
                    let end = start + 1000;
                    let end = end.min(docs.len());
                    for i in start..end {
                        idx.insert(i as u64, &docs[i % docs.len()]);
                    }
                }));
            }

            // 8 search threads
            for thread_id in 0..8_u64 {
                let idx = Arc::clone(&index);
                let qs = queries.clone();
                handles.push(std::thread::spawn(move || {
                    let start = (thread_id as usize * 12) % qs.len();
                    for qi in 0..12 {
                        let q = &qs[(start + qi) % qs.len()];
                        let _ = sparse_search(&idx, q, 10);
                    }
                }));
            }

            for h in handles {
                h.join().expect("thread panicked");
            }

            // Verify no deadlock occurred — index is still usable
            assert!(index.doc_count() > 0);
        });
    });

    group.finish();
}

criterion_group!(
    sparse_benches,
    sparse_insert_benchmarks,
    sparse_search_benchmarks,
    sparse_concurrent_benchmarks
);
criterion_main!(sparse_benches);
