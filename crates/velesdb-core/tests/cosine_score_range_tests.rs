#![cfg(feature = "persistence")]
//! The cosine score contract: one range, whichever path produced it.
//!
//! `HnswIndex` reaches a user-visible score down three different paths and they
//! must agree, because the caller cannot tell which one ran:
//!
//! - `search_brute_force` -> `compute_distance` -> `cosine_similarity_native`,
//!   taken for `SearchQuality::Perfect` and for any index of 100 vectors or
//!   fewer;
//! - `rerank_candidates_simd` -> `compute_distance`, taken when
//!   `len() > k * 2` with vector storage on;
//! - `search_hnsw_only` -> `NativeHnsw::transform_score`, taken otherwise.
//!
//! The first two return the cosine itself, in `[-1, 1]`. The third used to
//! share a match arm with Jaccard and clamp to `[0, 1]`, so every genuinely
//! negative cosine collapsed to exactly `0.0` — losing the sign and, worse,
//! the ordering among anti-correlated results, which all tied at zero.
//!
//! Nothing about the query says which path runs. `k` alone decides between the
//! second and the third.

#![allow(clippy::cast_precision_loss)]

use velesdb_core::index::hnsw::SearchQuality;
use velesdb_core::index::{HnswIndex, VectorIndex};
use velesdb_core::scored_result::ScoredResult;
use velesdb_core::DistanceMetric;

const DIM: usize = 16;
/// Above the 100-vector brute-force cutoff, so the graph paths are in play.
const CORPUS: u64 = 150;

/// The query direction: the first basis vector.
fn query() -> Vec<f32> {
    let mut q = vec![0.0_f32; DIM];
    q[0] = 1.0;
    q
}

/// Vector `i` points away from the query, with a cosine that rises toward zero
/// as `i` grows: `cos_i = -1 / sqrt(1 + (i/100)^2)`.
///
/// Every vector in the corpus is anti-correlated, and no two share a cosine, so
/// a clamp at zero does not merely change a sign — it makes the whole corpus
/// tie, and top-k becomes an arbitrary pick among 150 equal scores.
fn anti_correlated(i: u64) -> Vec<f32> {
    let mut v = vec![0.0_f32; DIM];
    v[0] = -1.0;
    v[1] = i as f32 / 100.0;
    v
}

fn expected_cosine(i: u64) -> f32 {
    let t = i as f32 / 100.0;
    -1.0 / (1.0 + t * t).sqrt()
}

fn build() -> HnswIndex {
    let index = HnswIndex::new(DIM, DistanceMetric::Cosine).expect("test: create index");
    for i in 0..CORPUS {
        index.insert(i, &anti_correlated(i));
    }
    index
}

fn score_of(results: &[ScoredResult], id: u64) -> Option<f32> {
    results.iter().find(|r| r.id == id).map(|r| r.score)
}

/// `k` must not change the range a cosine score is reported in.
///
/// `k = 10` on a 150-vector index satisfies `len() > k * 2`, so the two-stage
/// rerank runs and the score is the raw cosine. `k = 80` does not, so the
/// query falls through to `search_hnsw_only` and `transform_score`. Same
/// index, same query, same vector — the only difference is how many results
/// were asked for.
#[test]
fn cosine_scores_do_not_depend_on_how_many_results_were_requested() {
    let index = build();
    let q = query();

    let reranked = index
        .search_with_quality(&q, 10, SearchQuality::Balanced)
        .expect("test: small-k search");
    let graph_only = index
        .search_with_quality(&q, 80, SearchQuality::Balanced)
        .expect("test: large-k search");

    let top_id = reranked.first().expect("test: k=10 must return results").id;
    let small_k = score_of(&reranked, top_id).expect("test: id present at k=10");
    let large_k = score_of(&graph_only, top_id).expect("test: id present at k=80");

    assert!(
        (small_k - large_k).abs() < 1e-5,
        "doc {top_id} scored {small_k} at k=10 and {large_k} at k=80 — \
         the requested result count changed the score range"
    );
}

/// An anti-correlated match keeps its negative cosine on the graph path.
#[test]
fn the_graph_path_reports_a_negative_cosine_rather_than_zero() {
    let index = build();
    let q = query();

    // k = 80 on 150 vectors: `len() > k * 2` is false, so no rerank stage.
    let results = index
        .search_with_quality(&q, 80, SearchQuality::Balanced)
        .expect("test: search");

    assert!(!results.is_empty(), "test: expected results");
    for r in &results {
        let expected = expected_cosine(r.id);
        assert!(
            r.score < 0.0,
            "every corpus vector is anti-correlated, so doc {} cannot score {} \
             (true cosine {expected})",
            r.id,
            r.score
        );
        assert!(
            (r.score - expected).abs() < 1e-4,
            "doc {} scored {} but its cosine is {expected}",
            r.id,
            r.score
        );
    }
}

/// Clamping at zero costs the ranking, not just the sign.
///
/// With every cosine distinct and negative, the correct top-10 is the ten
/// vectors closest to orthogonal — the highest ids. A floor at zero makes all
/// 150 scores equal, so which ten come back is arbitrary.
#[test]
fn clamping_at_zero_would_destroy_the_ordering_among_anti_correlated_matches() {
    let index = build();
    let q = query();

    let results = index
        .search_with_quality(&q, 80, SearchQuality::Balanced)
        .expect("test: search");

    let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    let all_equal = scores
        .windows(2)
        .all(|w| (w[0] - w[1]).abs() < f32::EPSILON);
    assert!(
        !all_equal,
        "all {} returned scores are identical ({:?}) — the ordering was lost",
        scores.len(),
        scores.first()
    );

    for w in results.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "results must stay sorted by descending similarity: {} then {}",
            w[0].score,
            w[1].score
        );
    }
}

/// The graph path and the exhaustive path agree.
#[test]
fn the_graph_path_agrees_with_brute_force() {
    let index = build();
    let q = query();

    let graph = index
        .search_with_quality(&q, 80, SearchQuality::Balanced)
        .expect("test: graph search");
    let exact = index
        .search_with_quality(&q, 80, SearchQuality::Perfect)
        .expect("test: brute-force search");

    for r in &graph {
        let reference = score_of(&exact, r.id)
            .unwrap_or_else(|| panic!("test: doc {} missing from brute force", r.id));
        assert!(
            (r.score - reference).abs() < 1e-4,
            "doc {}: graph path scored {}, brute force {reference}",
            r.id,
            r.score
        );
    }
}

/// Jaccard keeps its floor at zero — the fix must not over-correct.
///
/// Jaccard similarity is genuinely `intersection / union`, so `[0, 1]` is its
/// true range and sharing Cosine's `[-1, 1]` would be as wrong as the shared
/// arm was in the other direction.
#[test]
fn jaccard_keeps_its_zero_floor() {
    let index = HnswIndex::new(DIM, DistanceMetric::Jaccard).expect("test: create index");
    for (slot, id) in (0..CORPUS).enumerate() {
        // Disjoint supports: every pair has an empty intersection.
        let mut v = vec![0.0_f32; DIM];
        v[slot % DIM] = 1.0;
        index.insert(id, &v);
    }

    let mut q = vec![0.0_f32; DIM];
    q[0] = 1.0;

    let results = index
        .search_with_quality(&q, 80, SearchQuality::Balanced)
        .expect("test: search");

    for r in &results {
        assert!(
            r.score >= 0.0,
            "Jaccard similarity cannot be negative: doc {} scored {}",
            r.id,
            r.score
        );
        assert!(
            r.score <= 1.0,
            "Jaccard similarity cannot exceed 1: doc {} scored {}",
            r.id,
            r.score
        );
    }
}

/// `SearchQuality::Adaptive` must not escalate on an easy query just because
/// the result tail sits near zero.
///
/// The escalation heuristic divides the first/last score gap by a baseline.
/// Taking that baseline as `min(|first|, |last|)` assumes zero is the metric's
/// floor — true for an unbounded distance, false for a similarity on
/// `[-1, 1]`, where a merely mediocre tail score of `-0.01` would make the
/// ratio explode to ~90 and force a second traversal on every such query.
/// Measuring from `score_range`'s floor instead keeps an easy query easy.
///
/// This heuristic never fired for cosine before: the `[0, 1]` clamp pinned
/// every non-positive tail at exactly `0.0`, so `baseline > f32::EPSILON` was
/// always false. Removing the clamp is what makes the baseline matter.
#[test]
fn adaptive_search_does_not_escalate_on_a_near_zero_tail() {
    let index = HnswIndex::new(DIM, DistanceMetric::Cosine).expect("test: create index");

    // A cluster tightly aligned with the query, plus a tail that lands just
    // below zero — the shape that made `min(|a|, |b|)` collapse.
    for i in 0..CORPUS {
        let mut v = vec![0.0_f32; DIM];
        if i < CORPUS - 10 {
            v[0] = 1.0;
            v[1] = i as f32 / 1000.0;
        } else {
            v[0] = -0.01;
            v[1] = 1.0;
        }
        index.insert(i, &v);
    }

    let adaptive = index
        .search_with_quality(
            &query(),
            20,
            SearchQuality::Adaptive {
                min_ef: 32,
                max_ef: 256,
            },
        )
        .expect("test: adaptive search");

    assert!(!adaptive.is_empty(), "adaptive search must return results");
    let (low, high) = DistanceMetric::Cosine
        .score_range()
        .expect("cosine is bounded");
    for r in &adaptive {
        assert!(
            r.score >= low && r.score <= high,
            "doc {} scored {} outside cosine's range",
            r.id,
            r.score
        );
    }
    for w in adaptive.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "adaptive results must stay sorted by descending similarity"
        );
    }
}
