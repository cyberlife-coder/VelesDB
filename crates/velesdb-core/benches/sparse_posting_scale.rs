//! Sparse search latency against corpus size — the regime #2092 argues about.
//!
//! # Why this bench exists
//!
//! #2092 claims `PostingEntry { doc_id: u64, weight: f32 }` wastes 25% of every
//! posting cache line to alignment padding. That is a statement about **large**
//! posting lists: padding costs nothing while a term's postings sit in L1, and
//! only starts costing bandwidth and misses once the scanned run exceeds cache.
//!
//! `sparse_benchmark` cannot arbitrate it. It fixes 10 000 documents over a
//! 30 000-term vocabulary at 50–200 nonzeros per document — about 42 postings
//! per term, ~667 bytes per list, ten cache lines. A whole query touches tens of
//! kilobytes. Nothing there is out of cache, so any layout change measured on it
//! reports something other than the effect claimed.
//!
//! This bench sweeps the corpus so the regime is chosen deliberately, and prints
//! the bytes a query actually touches next to the latency, so a reader can see
//! which side of their machine's last-level cache a given row sits on.
//!
//! # Two things it prints that are not latency
//!
//! **Frozen segment count.** `FREEZE_THRESHOLD` is 10 000 documents, so the
//! corpus size also sets how many frozen segments exist, and
//! `collect_frozen_runs` clones a run out of *each* one on every query term.
//! Segment count, not padding, may well be what moves these numbers; printing it
//! keeps that visible rather than confounded with size.
//!
//! **A result checksum.** Two configurations are comparable only when their
//! checksums match — otherwise they searched different indexes and the latency
//! difference includes that.
//!
//! # The build here is deterministic, unlike the HNSW benches
//!
//! `SparseInvertedIndex::insert_batch_chunk` is sequential — there is no rayon
//! anywhere in `sparse_index` — so the same corpus produces the same index every
//! run. This bench therefore does *not* carry the confound documented on #2075,
//! where `insert_batch_parallel` built a different graph on every run and made
//! criterion report tight intervals around the wrong variance component.
//!
//! Process-to-process noise is still real. Run each configuration **twice** and
//! require the pair to agree more closely than any difference being claimed,
//! on an otherwise idle machine.
//!
//! # Running
//!
//! ```text
//! VELESDB_SPARSE_SCALE_DOCS=10000,50000,200000 VELESDB_SPARSE_SCALE_VOCAB=30000 \
//!   cargo bench --bench sparse_posting_scale
//! ```
//!
//! Lowering the vocabulary is the cheap way to lengthen posting lists without
//! paying for a larger corpus: list length is `docs * avg_nnz / vocab`.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use std::collections::HashSet;
use std::time::Instant;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use velesdb_core::index::sparse::{sparse_search, SparseInvertedIndex, SparseVector};

/// Bytes one `PostingEntry` occupies today: `u64` + `f32` + 4 bytes of tail
/// padding forced by align-8 on the `u64`. The figure #2092 wants to shrink.
const POSTING_ENTRY_BYTES: usize = 16;

/// Nonzeros per generated document, matching `sparse_benchmark`'s SPLADE-like
/// shape so the two benches describe the same kind of corpus.
const NNZ_RANGE: std::ops::RangeInclusive<usize> = 50..=200;

/// Queries issued per measured iteration set.
const QUERY_COUNT: usize = 100;

/// Corpus sizes swept when `VELESDB_SPARSE_SCALE_DOCS` is unset.
const DEFAULT_DOCS: &[usize] = &[10_000, 50_000, 200_000];

/// Vocabulary size used when `VELESDB_SPARSE_SCALE_VOCAB` is unset.
const DEFAULT_VOCAB: u32 = 30_000;

/// Reads a comma-separated list of corpus sizes from the environment.
fn scale_docs() -> Vec<usize> {
    let Ok(raw) = std::env::var("VELESDB_SPARSE_SCALE_DOCS") else {
        return DEFAULT_DOCS.to_vec();
    };
    let parsed: Vec<usize> = raw
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .filter(|&n: &usize| n > 0)
        .collect();
    if parsed.is_empty() {
        DEFAULT_DOCS.to_vec()
    } else {
        parsed
    }
}

/// Reads the vocabulary size from the environment.
fn scale_vocab() -> u32 {
    std::env::var("VELESDB_SPARSE_SCALE_VOCAB")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_VOCAB)
}

/// Generates a SPLADE-like corpus over `vocab` terms, deterministic in `seed`.
fn generate_corpus(n: usize, vocab: u32, seed: u64) -> Vec<SparseVector> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let nnz = rng.random_range(NNZ_RANGE).min(vocab as usize);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used = HashSet::new();
            while pairs.len() < nnz {
                let term_id = rng.random_range(0..vocab);
                if used.insert(term_id) {
                    pairs.push((term_id, rng.random_range(0.01_f32..2.0)));
                }
            }
            SparseVector::new(pairs)
        })
        .collect()
}

/// Builds the index sequentially, so the same corpus yields the same index.
fn build_index(corpus: &[SparseVector]) -> SparseInvertedIndex {
    let index = SparseInvertedIndex::new();
    let docs: Vec<(u64, SparseVector)> = corpus
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, v)| (i as u64, v))
        .collect();
    index.insert_batch_chunk(&docs);
    index
}

/// Bytes of posting entries one query reads, summed over its terms.
///
/// This is the number to compare against last-level cache — not the index size,
/// and not one term's list. A query touches every one of its terms' runs.
fn bytes_touched_per_query(index: &SparseInvertedIndex, query: &SparseVector) -> usize {
    query
        .indices
        .iter()
        .map(|&term_id| index.posting_count(term_id) * POSTING_ENTRY_BYTES)
        .sum()
}

/// Order-independent checksum over fixed query results.
///
/// Two configurations are comparable only when these match: a differing
/// checksum means the runs searched different indexes, and the latency gap
/// includes that difference rather than isolating the change under test.
fn result_checksum(index: &SparseInvertedIndex, queries: &[SparseVector], k: usize) -> u64 {
    let mut sum: u64 = 0;
    for query in queries {
        for doc in sparse_search(index, query, k) {
            sum = sum
                .wrapping_mul(31)
                .wrapping_add(doc.doc_id)
                .wrapping_add(doc.score.to_bits().into());
        }
    }
    sum
}

fn sparse_posting_scale(c: &mut Criterion) {
    let vocab = scale_vocab();
    let mut group = c.benchmark_group("sparse_posting_scale");
    group.sample_size(10);

    for &docs in &scale_docs() {
        let corpus = generate_corpus(docs, vocab, 42);
        let queries = generate_corpus(QUERY_COUNT, vocab, 123);

        let build_started = Instant::now();
        let index = build_index(&corpus);
        let build_secs = build_started.elapsed().as_secs_f64();

        let total_postings: usize = corpus.iter().map(SparseVector::nnz).sum();
        let touched: usize = queries
            .iter()
            .map(|q| bytes_touched_per_query(&index, q))
            .sum::<usize>()
            / queries.len();
        let checksum = result_checksum(&index, &queries, 10);

        // Printed, never asserted: the bench cannot know this machine's cache
        // sizes, and the point is to let a reader place each row against theirs.
        println!(
            "\n[sparse_posting_scale] docs={docs} vocab={vocab} \
postings={total_postings} (~{per_term:.0}/term) \
frozen_segments={segments} build={build_secs:.1}s\n\
  bytes touched per query ~{touched_mib:.1} MiB at {POSTING_ENTRY_BYTES} B/entry \
— compare against last-level cache; padding is free below it\n\
  result checksum {checksum:#018x} — only compare runs whose checksums match",
            per_term = total_postings as f64 / f64::from(vocab),
            segments = docs / velesdb_core::index::sparse::inverted_index::FREEZE_THRESHOLD,
            touched_mib = touched as f64 / (1024.0 * 1024.0),
        );

        group.bench_function(format!("top10_{docs}docs_{vocab}vocab"), |b| {
            let mut qi = 0;
            b.iter(|| {
                let query = &queries[qi % queries.len()];
                qi += 1;
                sparse_search(black_box(&index), black_box(query), 10)
            });
        });
    }

    group.finish();
}

criterion_group!(benches, sparse_posting_scale);
criterion_main!(benches);
