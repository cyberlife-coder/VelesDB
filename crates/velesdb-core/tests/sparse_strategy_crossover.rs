#![cfg(all(test, feature = "internal-bench"))]
//! Does `MaxScore` earn its keep, and where? (#2177)
//!
//! `sparse_search` sends everything at or below `SMALL_CORPUS_LINEAR_THRESHOLD`
//! (= `MAX_DENSE_ACCUMULATOR`, 1 000 000) to the linear scan, and only above it
//! can the coverage heuristic reach `maxscore_search`. The source says plainly
//! that the branch above is unmeasured, and #2177 keeps the retirement
//! question open for exactly that reason: nobody has scored one corpus both
//! ways at one size.
//!
//! The router cannot be asked for the other strategy at a given size, so this
//! harness calls both through the `internal-bench` seam and compares the
//! **work**, in posting inspections (`sparse_index::op_count`), never a timer.
//! That is what `cost_crossover.rs` established for the dense dispatch and why
//! its numbers survive a shared runner: the corpus here is closed-form with no
//! RNG, so every count below is bit-for-bit reproducible across machines.
//!
//! # Sizes
//!
//! The default sweep is small enough for CI. The decision #2177 waits on needs
//! sizes above the threshold, which build a multi-gigabyte corpus and belong on
//! a workstation:
//!
//! ```text
//! VELESDB_SPARSE_CROSSOVER_DOCS=200000,1100000 \
//!   cargo test --release -p velesdb-core --features internal-bench,persistence \
//!   --test sparse_strategy_crossover -- --nocapture
//! ```
//!
//! What this harness asserts is what holds at any size: the two strategies
//! return the same documents, and the counter actually moved. It deliberately
//! does **not** assert which one is cheaper — that is the open question, and a
//! test that fixed the answer at a CI-sized corpus would be pinning the wrong
//! size.

use velesdb_core::index::sparse::{SparseInvertedIndex, SparseVector};
use velesdb_core::internal_bench::{
    reset_sparse_scoring_ops, sparse_linear_scan_search, sparse_maxscore_search, sparse_scoring_ops,
};

/// Corpus sizes swept when `VELESDB_SPARSE_CROSSOVER_DOCS` is unset.
const DEFAULT_DOCS: &[usize] = &[10_000, 50_000];
/// Vocabulary the corpus draws terms from.
const VOCAB: u32 = 30_000;
/// Nonzeros per document, matching the SPLADE-like shape the other sparse
/// benches use so the corpora describe the same kind of index.
const DOC_NNZ: usize = 60;
/// Queries scored per configuration.
const QUERIES: usize = 8;
/// Result size.
const K: usize = 10;
/// Tolerance when comparing the two strategies' scores.
///
/// They accumulate the same products in different orders — the linear scan
/// term by term, `MaxScore` cursor by cursor — so the last bits of an `f32`
/// sum
/// need not match. Wide enough for that reassociation, far tighter than any
/// difference a wrongly pruned document would produce.
const SCORE_EPSILON: f32 = 1e-4;

/// A closed-form pseudo-random term id: no RNG, so the corpus is identical on
/// every machine and the counts below are comparable across commits.
fn term_of(doc: usize, slot: usize) -> u32 {
    let mixed = (doc as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (slot as u64).wrapping_mul(0x51_7C_C1_B7);
    u32::try_from(mixed % u64::from(VOCAB)).expect("modulo VOCAB fits a u32")
}

/// Weight in `[0.25, 1.0)`, derived from the same closed form.
fn weight_of(doc: usize, slot: usize) -> f32 {
    let mixed = (doc as u64).wrapping_mul(0x2545_F491_4F6C_DD1D) ^ (slot as u64);
    // `% 1024` bounds this to 0..=1023, which a u16 holds exactly and an f32
    // represents exactly — no precision is lost, and going through u16 says so
    // rather than asking a reader to trust an `as f32` on a u64.
    let bucket = u16::try_from(mixed % 1024).expect("test: modulo 1024 fits a u16");
    0.25 + f32::from(bucket) / 1365.0
}

fn document(doc: usize) -> SparseVector {
    let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(DOC_NNZ);
    let mut seen = std::collections::HashSet::with_capacity(DOC_NNZ);
    for slot in 0..DOC_NNZ {
        let term = term_of(doc, slot);
        if seen.insert(term) {
            pairs.push((term, weight_of(doc, slot)));
        }
    }
    pairs.sort_unstable_by_key(|&(t, _)| t);
    SparseVector::new(pairs)
}

/// The two query shapes #2177 distinguishes.
#[derive(Clone, Copy)]
enum Shape {
    /// 125 terms, uniform weights — `MaxScore`'s worst case, because its upper
    /// bounds need a score separation uniform weights deny it.
    Uniform,
    /// 30 terms, geometrically decaying weights — the SPLADE-like shape a
    /// production workload issues, and the one pruning can actually use.
    Skewed,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::Uniform => "uniform",
            Self::Skewed => "skewed",
        }
    }

    fn query(self, q: usize) -> SparseVector {
        let (nnz, decay) = match self {
            Self::Uniform => (125usize, 1.0_f32),
            Self::Skewed => (30usize, 0.82_f32),
        };
        let mut pairs: Vec<(u32, f32)> = Vec::with_capacity(nnz);
        let mut seen = std::collections::HashSet::with_capacity(nnz);
        let mut w = 2.0_f32;
        for slot in 0..nnz {
            let term = term_of(q.wrapping_mul(7919) + 13, slot);
            if seen.insert(term) {
                pairs.push((term, w));
            }
            w *= decay;
        }
        pairs.sort_unstable_by_key(|&(t, _)| t);
        SparseVector::new(pairs)
    }
}

fn build(docs: usize) -> SparseInvertedIndex {
    let index = SparseInvertedIndex::new();
    let batch: Vec<(u64, SparseVector)> = (0..docs).map(|d| (d as u64, document(d))).collect();
    for chunk in batch.chunks(10_000) {
        index.insert_batch_chunk(chunk);
    }
    index
}

/// Posting inspections one strategy spends on `queries`, and the scores it
/// returns for the first of them.
fn measure(
    search: impl Fn(
        &SparseInvertedIndex,
        &SparseVector,
        usize,
    ) -> Vec<velesdb_core::index::sparse::ScoredDoc>,
    index: &SparseInvertedIndex,
    queries: &[SparseVector],
) -> (u64, Vec<f32>) {
    reset_sparse_scoring_ops();
    let mut first: Vec<f32> = Vec::new();
    for (i, q) in queries.iter().enumerate() {
        let hits = search(index, q, K);
        if i == 0 {
            first = hits.iter().map(|h| h.score).collect();
        }
    }
    (sparse_scoring_ops(), first)
}

fn sizes() -> Vec<usize> {
    match std::env::var("VELESDB_SPARSE_CROSSOVER_DOCS") {
        Ok(v) => v.split(',').filter_map(|s| s.trim().parse().ok()).collect(),
        Err(_) => DEFAULT_DOCS.to_vec(),
    }
}

#[test]
fn maxscore_and_linear_scan_cost_the_same_corpus_differently() {
    println!("{{\"harness\":\"sparse_strategy_crossover\",\"unit\":\"posting inspections\"}}");
    for &docs in &sizes() {
        let index = build(docs);
        assert_eq!(index.doc_count(), docs as u64, "corpus size as requested");

        for shape in [Shape::Uniform, Shape::Skewed] {
            let queries: Vec<SparseVector> = (0..QUERIES).map(|q| shape.query(q)).collect();

            let (linear_ops, linear_scores) = measure(sparse_linear_scan_search, &index, &queries);
            let (maxscore_ops, maxscore_scores) = measure(sparse_maxscore_search, &index, &queries);

            // A work measure that never moved would let this harness report a
            // ratio of nothing over nothing and pass.
            assert!(linear_ops > 0, "linear scan recorded no work at {docs}");
            assert!(maxscore_ops > 0, "maxscore recorded no work at {docs}");

            // The seam must not have changed what the strategies answer: the
            // comparison is only meaningful between two correct searches.
            // The strategies must agree on the SCORES, position by position.
            //
            // Not on the document ids: two documents can score identically at
            // the k-th place, and the two heaps then keep different ones —
            // measured here at 50 000 docs, skewed, where `14907` and `41247`
            // both score 1.820146561. That is a tie, not a disagreement, and
            // asserting ids would have reported a correctness bug where there
            // is none. Comparing scores catches a genuinely wrong result — a
            // MaxScore bound that pruned a document it should have kept would
            // show up as a lower score at that position — without failing on
            // which of two equals won.
            assert_eq!(
                linear_scores.len(),
                maxscore_scores.len(),
                "{} at {docs} docs: different result counts",
                shape.name()
            );
            for (i, (l, m)) in linear_scores.iter().zip(&maxscore_scores).enumerate() {
                assert!(
                    (l - m).abs() <= SCORE_EPSILON,
                    "{} at {docs} docs: rank {i} scores differ, linear {l} vs maxscore {m}",
                    shape.name()
                );
            }

            #[allow(clippy::cast_precision_loss)]
            let ratio = maxscore_ops as f64 / linear_ops as f64;
            println!(
                "{{\"docs\":{docs},\"shape\":\"{}\",\"linear_ops\":{linear_ops},\
                 \"maxscore_ops\":{maxscore_ops},\"maxscore_over_linear\":{ratio:.2}}}",
                shape.name()
            );
        }
    }
}
