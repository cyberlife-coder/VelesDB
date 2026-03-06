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

use velesdb_core::index::sparse::{sparse_search, SparseInvertedIndex, SparseVector};

/// Generate a corpus of SPLADE-like sparse vectors.
///
/// Each vector has 50-200 nonzero entries with term IDs in `0..30_000`
/// and weights uniformly sampled from `0.01..2.0`.
fn generate_splade_corpus(n: usize, seed: u64) -> Vec<SparseVector> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let nnz = rng.gen_range(50..=200);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used = HashSet::new();
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
    group.bench_function("parallel_10k_4threads", |b| {
        use rayon::prelude::*;
        b.iter(|| {
            let index = Arc::new(SparseInvertedIndex::new());
            corpus.par_iter().enumerate().for_each(|(i, vec)| {
                index.insert(black_box(i as u64), black_box(vec));
            });
            index
        });
    });

    group.finish();
}

fn sparse_search_benchmarks(c: &mut Criterion) {
    let corpus = generate_splade_corpus(10_000, 42);
    let queries = generate_splade_corpus(100, 123);
    let index = build_index(&corpus);

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
