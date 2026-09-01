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
//!
//! # A trap inside this bench's own sweep range
//!
//! `sparse_search` picks its strategy from `doc_count`:
//! `SMALL_CORPUS_LINEAR_THRESHOLD` is 100 000 documents, below which every
//! query takes `linear_scan_search` and above which it can reach
//! `maxscore_search`. That boundary sits **inside** the range this bench
//! sweeps, and crossing it dominates everything else.
//!
//! Measured here, two passes each, checksums matching: 90 000 documents ran in
//! 537 µs and 522 µs; 110 000 ran in 22.8 ms and 23.0 ms. Twenty-two percent
//! more data, forty-two times the latency — the jump is at the threshold, not
//! in the volume.
//!
//! So a comparison that straddles 100 000 documents measures the strategy
//! switch, not whatever change is under test. **Keep both arms of any A/B on
//! the same side of that boundary**, and read a size sweep that crosses it as
//! two separate curves rather than one.
//!
//! # Query shape (#2177 step 1)
//!
//! `VELESDB_SPARSE_SCALE_QUERY_SHAPE` selects the query generator:
//!
//! - `uniform` (default): 50–200 terms, uniform weights — inherited from
//!   `sparse_benchmark`, and the worst case for `MaxScore` pruning, whose
//!   upper bounds need the score separation uniform weights deny it.
//! - `skewed`: 20–50 terms with geometrically decaying weights — the shape
//!   of a real SPLADE query, which is what #2177's step 1 asks to measure.
//!
//! The corpus generator is shared by both shapes, so the index under test is
//! identical; only the queries differ. Checksums are therefore comparable
//! only within one shape — across shapes they are different queries by
//! construction, and the header line names the shape so a run cannot be
//! misfiled.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use std::collections::HashSet;
use std::time::Instant;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use velesdb_core::index::sparse::{sparse_search, PostingEntry, SparseInvertedIndex, SparseVector};

/// Bytes one `PostingEntry` occupies — asked of the compiler, never written
/// down.
///
/// This bench exists to arbitrate #2092, whose proposal is to shrink this
/// exact struct. A literal here would keep reporting the pre-change footprint
/// for post-change code: the `bytes touched per query` line — the one figure
/// this file is built to produce — would read identically before and after the
/// shrink, and an A/B would conclude the layout change did nothing. The
/// instrument would be blind to the only change it exists to measure.
///
/// Same defect class as #2165, where the adjacency bench printed a width it
/// could not observe. There the neighbour id type was `pub(crate)` and the
/// honest fix was to print both plausible widths; `PostingEntry` is `pub` and
/// reachable through the path this file already imports, so here the compiler
/// can simply be asked.
const POSTING_ENTRY_BYTES: usize = std::mem::size_of::<PostingEntry>();

/// Nonzeros per generated document, matching `sparse_benchmark`'s SPLADE-like
/// shape so the two benches describe the same kind of corpus.
const NNZ_RANGE: std::ops::RangeInclusive<usize> = 50..=200;

/// Queries issued per measured iteration set.
const QUERY_COUNT: usize = 100;

/// Corpus sizes swept when `VELESDB_SPARSE_SCALE_DOCS` is unset.
const DEFAULT_DOCS: &[usize] = &[10_000, 50_000, 200_000];

/// Vocabulary size used when `VELESDB_SPARSE_SCALE_VOCAB` is unset.
const DEFAULT_VOCAB: u32 = 30_000;

/// Spacing between consecutive document ids when `VELESDB_SPARSE_SCALE_ID_STRIDE`
/// is unset. 1 gives the compact `0..n` space every other sweep here uses.
const DEFAULT_ID_STRIDE: u64 = 1;

/// `MAX_DENSE_ACCUMULATOR` as it read when this bench was written.
///
/// Reported, never used to decide anything. The real constant is private to
/// `sparse_index::search`, so a bench genuinely cannot observe which
/// accumulator a run took — it can only print the two inputs it does know and
/// name the third. Asserting a regime it cannot see is how a header comes to
/// lie about its own configuration, which is the defect #2165 fixed on
/// `hnsw_adjacency_scale` and which the first draft of this line repeated.
const DENSE_CAP_AT_WRITING: u64 = 1_000_000;

/// Nonzeros per skewed-shape query: real SPLADE queries carry far fewer
/// terms than documents do (#2177 step 1).
const SKEWED_QUERY_NNZ_RANGE: std::ops::RangeInclusive<usize> = 20..=50;

/// First-rank weight of a skewed query; later ranks decay geometrically.
const SKEWED_MAX_WEIGHT: f32 = 2.0;

/// Per-rank geometric decay of skewed query weights: rank 0 gets 2.0, rank
/// 49 gets ~0.011 — the strong separation `MaxScore`'s pruning feeds on and
/// uniform weights deny it.
const SKEWED_WEIGHT_DECAY: f32 = 0.9;

/// Which query generator a run uses. Selected by
/// `VELESDB_SPARSE_SCALE_QUERY_SHAPE`; anything but `skewed` (including
/// unset) is the historical uniform shape, so existing runs stay comparable.
#[derive(Clone, Copy)]
enum QueryShape {
    Uniform,
    Skewed,
}

impl QueryShape {
    fn from_env() -> Self {
        match std::env::var("VELESDB_SPARSE_SCALE_QUERY_SHAPE").as_deref() {
            Ok("skewed") => Self::Skewed,
            _ => Self::Uniform,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Uniform => "uniform",
            Self::Skewed => "skewed",
        }
    }
}

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

/// Reads the document-id stride from the environment.
///
/// This knob exists to reach `linear_scan_search`'s **other** accumulator.
/// The dense-versus-hashmap choice is not made on corpus size but on id
/// compactness:
///
/// ```text
/// use_dense = max_doc_id <= MAX_DENSE_ACCUMULATOR
///          && max_doc_id < doc_count * 4
/// ```
///
/// so a stride of 4 or more puts `max_doc_id` past `doc_count * 4` and routes
/// to `linear_scan_hashmap` at any corpus size, with the same number of
/// postings and the same memory as the compact run. Without this, every sweep
/// measures the dense accumulator only, and the hashmap variant — the one
/// #2182's routing change reaches but never measured — stays invisible.
///
/// **Checksums do not carry across strides, and that is not a defect.** The
/// checksum folds `doc_id`, so relabelling ids necessarily changes it. What is
/// preserved is the thing the comparison needs: scores depend on weights
/// alone, and `i * stride` is order-preserving, so a stride run retrieves the
/// same documents in the same rank order with every id multiplied by the
/// stride. Compare checksums between runs at equal stride — never across two.
fn scale_id_stride() -> u64 {
    std::env::var("VELESDB_SPARSE_SCALE_ID_STRIDE")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_ID_STRIDE)
}

/// Generates sparse vectors over `vocab` terms, deterministic in `seed`.
///
/// `weight(rng, rank)` decides each nonzero's weight from its draw rank, so
/// one skeleton serves both the uniform corpus/query shape and the skewed
/// query shape without duplicating the dedup loop.
fn generate_vectors(
    n: usize,
    vocab: u32,
    seed: u64,
    nnz_range: std::ops::RangeInclusive<usize>,
    weight: impl Fn(&mut StdRng, usize) -> f32,
) -> Vec<SparseVector> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let nnz = rng.random_range(nnz_range.clone()).min(vocab as usize);
            let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
            let mut used = HashSet::new();
            while pairs.len() < nnz {
                let term_id = rng.random_range(0..vocab);
                if used.insert(term_id) {
                    let w = weight(&mut rng, pairs.len());
                    pairs.push((term_id, w));
                }
            }
            SparseVector::new(pairs)
        })
        .collect()
}

/// Generates a SPLADE-like corpus over `vocab` terms, deterministic in `seed`.
fn generate_corpus(n: usize, vocab: u32, seed: u64) -> Vec<SparseVector> {
    generate_vectors(n, vocab, seed, NNZ_RANGE, |rng, _rank| {
        rng.random_range(0.01_f32..2.0)
    })
}

/// Generates queries for the selected shape, deterministic in `seed`.
fn generate_queries(shape: QueryShape, n: usize, vocab: u32, seed: u64) -> Vec<SparseVector> {
    match shape {
        QueryShape::Uniform => generate_corpus(n, vocab, seed),
        QueryShape::Skewed => {
            generate_vectors(n, vocab, seed, SKEWED_QUERY_NNZ_RANGE, |_rng, rank| {
                SKEWED_MAX_WEIGHT
                    * SKEWED_WEIGHT_DECAY.powi(i32::try_from(rank).unwrap_or(i32::MAX))
            })
        }
    }
}

/// Builds the index sequentially, so the same corpus yields the same index.
fn build_index(corpus: &[SparseVector], id_stride: u64) -> SparseInvertedIndex {
    let index = SparseInvertedIndex::new();
    let docs: Vec<(u64, SparseVector)> = corpus
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, v)| (i as u64 * id_stride, v))
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

/// Order-sensitive checksum over fixed query results.
///
/// The fold multiplies before adding, so both the result set AND its ranking
/// must match — stricter than a set comparison, which is what an A/B of a
/// layout change wants. Two configurations are comparable only when these
/// match: a differing checksum means the runs searched different indexes (or
/// ranked differently), and the latency gap includes that difference rather
/// than isolating the change under test.
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
    let shape = QueryShape::from_env();
    let id_stride = scale_id_stride();
    let mut group = c.benchmark_group("sparse_posting_scale");
    group.sample_size(10);

    for &docs in &scale_docs() {
        let corpus = generate_corpus(docs, vocab, 42);
        let queries = generate_queries(shape, QUERY_COUNT, vocab, 123);

        let build_started = Instant::now();
        let index = build_index(&corpus, id_stride);
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
            "\n[sparse_posting_scale] docs={docs} vocab={vocab} query_shape={shape_label} \
postings={total_postings} (~{per_term:.0}/term) \
frozen_segments={segments} build={build_secs:.1}s\n\
  bytes touched per query ~{touched_mib:.1} MiB at {POSTING_ENTRY_BYTES} B/entry \
— compare against last-level cache; padding is free below it\n\
  id_stride={id_stride} max_doc_id={max_doc_id} vs doc_count*4={compact_cap} \
— dense also needs max_doc_id <= MAX_DENSE_ACCUMULATOR, a private constant \
this bench cannot read ({DENSE_CAP_AT_WRITING} when written)\n\
  result checksum {checksum:#018x} — only compare runs whose checksums match",
            shape_label = shape.label(),
            max_doc_id = (docs as u64 - 1) * id_stride,
            compact_cap = docs as u64 * 4,
            per_term = total_postings as f64 / f64::from(vocab),
            segments = docs / velesdb_core::index::sparse::inverted_index::FREEZE_THRESHOLD,
            touched_mib = touched as f64 / (1024.0 * 1024.0),
        );

        group.bench_function(
            format!(
                "top10_{docs}docs_{vocab}vocab_{}_stride{id_stride}",
                shape.label()
            ),
            |b| {
                let mut qi = 0;
                b.iter(|| {
                    let query = &queries[qi % queries.len()];
                    qi += 1;
                    sparse_search(black_box(&index), black_box(query), 10)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, sparse_posting_scale);
criterion_main!(benches);
