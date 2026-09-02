//! Codec-generic contract suites for the quantized-precision backends,
//! plus the fixtures and assertions they share.
//!
//! The `check_*` functions ARE the test bodies: each is generic over
//! [`TraversalCodec`], so every contract of the shared state machine (lazy
//! training, install alignment, rerank semantics, fallback guards, recall)
//! is written once here and pinned for each codec by a thin `#[test]`
//! wrapper in `rabitq_precision_tests` / `sq8_precision_tests`. The file
//! therefore carries no `#[test]` of its own — and is named `_tests.rs`
//! because it is test code: the production-panic gate
//! (`scripts/check_prod_unwraps.py`) classifies by filename, and a helper
//! module gated only at its `mod` declaration would otherwise be scanned
//! as production.

use super::distance::CachedSimdDistance;
use super::quantized_precision::{QuantizedPrecisionHnsw, TraversalCodec};
use crate::distance::DistanceMetric;
use std::collections::HashSet;
use std::sync::Arc;

type Backend<C> = QuantizedPrecisionHnsw<CachedSimdDistance, C>;

/// Builds a Euclidean backend for codec `C`.
fn euclidean_backend<C: TraversalCodec>(
    dim: usize,
    max_connections: usize,
    ef_construction: usize,
    max_elements: usize,
) -> Backend<C> {
    let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Euclidean, dim);
    Backend::<C>::new(engine, dim, max_connections, ef_construction, max_elements).expect("test")
}

/// Inserts the deterministic "ramp" vectors `range` (vector i = the
/// sequence `i*dim..i*dim+dim` as f32) — self-queries have exact matches.
fn insert_ramp<C: TraversalCodec>(hnsw: &Backend<C>, dim: usize, range: std::ops::Range<usize>) {
    for i in range {
        let v: Vec<f32> = (0..dim).map(|j| (i * dim + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }
}

/// Builds a force-trained backend over 100 planted unit vectors for
/// `metric` (query id 42), returning the backend and the vector set.
pub(super) fn trained_planted_backend<C: TraversalCodec>(
    metric: DistanceMetric,
    dim: usize,
) -> (Backend<C>, Vec<Vec<f32>>) {
    let engine = CachedSimdDistance::new_prenormalized(metric, dim);
    let hnsw = Backend::<C>::new(engine, dim, 16, 200, 1000).expect("test");
    let vectors = planted_unit_vectors(100, dim, PLANTED_QUERY_ID);
    for v in &vectors {
        hnsw.insert(v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");
    assert!(hnsw.is_quantizer_trained());
    (hnsw, vectors)
}

/// The self-query id used by [`trained_planted_backend`].
pub(super) const PLANTED_QUERY_ID: usize = 42;

/// Normalizes `v` to unit length in place.
fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in v.iter_mut() {
        *x /= norm;
    }
}

/// Deterministic pseudo-random unit vector (LCG-seeded).
///
/// Unit norm keeps Cosine and DotProduct orderings identical and ensures
/// the self-query is the unique maximum-similarity result.
pub(super) fn unit_vector(seed: u64, dim: usize) -> Vec<f32> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut v: Vec<f32> = (0..dim)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 40) as f32 / 8_388_608.0) - 1.0
        })
        .collect();
    normalize(&mut v);
    v
}

/// Generates `n` random unit vectors with 10 planted neighbors of
/// `query_id` in the last 10 slots.
///
/// The planted neighbors (similarity ~0.91..0.995) are well separated from
/// the near-orthogonal random background, so the brute-force top-10 is
/// unambiguous and within reach of coarse quantized traversal.
pub(super) fn planted_unit_vectors(n: usize, dim: usize, query_id: usize) -> Vec<Vec<f32>> {
    debug_assert!(query_id < n - 10, "query must not overlap planted slots");
    let mut vectors: Vec<Vec<f32>> = (0..n).map(|i| unit_vector(i as u64 + 1, dim)).collect();
    for slot in 0..10 {
        let noise = unit_vector(1_000 + slot as u64, dim);
        let eps = 0.1 + 0.04 * slot as f32;
        let mut v: Vec<f32> = vectors[query_id]
            .iter()
            .zip(noise.iter())
            .map(|(a, b)| a + eps * b)
            .collect();
        normalize(&mut v);
        vectors[n - 10 + slot] = v;
    }
    vectors
}

/// Builds `n` sinusoidal vectors of dimension `dim`.
pub(super) fn sinusoidal_vectors(n: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.01).sin())
                .collect()
        })
        .collect()
}

/// Brute-force top-k ids by exact metric similarity/distance.
///
/// Sorts independently of production code (explicit branch on
/// `higher_is_better`) so the assertion is not self-referential.
pub(super) fn brute_force_top_ids(
    vectors: &[Vec<f32>],
    query: &[f32],
    metric: DistanceMetric,
    k: usize,
) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (i, metric.calculate(query, v)))
        .collect();
    if metric.higher_is_better() {
        scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    } else {
        scored.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
    }
    scored.truncate(k);
    scored.into_iter().map(|(i, _)| i).collect()
}

/// Asserts the self-query ranks first with maximal similarity and that
/// recall@k vs brute-force is >= 0.95.
pub(super) fn assert_top1_and_recall(
    results: &[(usize, f32)],
    vectors: &[Vec<f32>],
    query_id: usize,
    metric: DistanceMetric,
    k: usize,
) {
    assert!(!results.is_empty(), "search returned no results");
    assert_eq!(
        results[0].0, query_id,
        "self-query must rank first for {metric:?}, got node {} (score {})",
        results[0].0, results[0].1
    );
    assert!(
        results[0].1 > 0.99,
        "self-similarity must be maximal for {metric:?}, got {}",
        results[0].1
    );

    let expected = brute_force_top_ids(vectors, &vectors[query_id], metric, k);
    let got: HashSet<usize> = results.iter().map(|(id, _)| *id).collect();
    let overlap = expected.iter().filter(|id| got.contains(id)).count();
    let recall = overlap as f64 / k as f64;
    assert!(
        recall >= 0.95,
        "recall@{k} vs brute-force must be >= 0.95 for {metric:?}, got {recall:.2}"
    );
}

// =========================================================================
// Generic behavior suites (one body per contract, one #[test] per codec)
// =========================================================================

/// A fresh index is empty, untrained, and returns no results.
pub(super) fn check_empty_index<C: TraversalCodec>(dim: usize) {
    let hnsw = euclidean_backend::<C>(dim, 16, 100, 1000);
    assert!(hnsw.is_empty());
    assert!(!hnsw.is_quantizer_trained());
    assert!(hnsw.search(&vec![0.0_f32; dim], 10, 50).is_empty());
}

/// Below the lazy-train threshold nothing trains, and search works via the
/// exact-f32 fallback (self-query of node 0 ranks first).
pub(super) fn check_untrained_fallback<C: TraversalCodec>(dim: usize) {
    let hnsw = euclidean_backend::<C>(dim, 16, 100, 1000);
    insert_ramp(&hnsw, dim, 0..50);
    assert_eq!(hnsw.len(), 50);
    assert!(!hnsw.is_quantizer_trained(), "Should not train yet");

    let query: Vec<f32> = (0..dim).map(|j| j as f32).collect();
    let results = hnsw.search(&query, 10, 50);
    assert!(!results.is_empty());
    assert_eq!(results[0].0, 0, "Closest should be node 0");
}

/// Crossing `training_sample_size` (min(1000, `max_elements`) = 100 here)
/// trains the quantizer from inserts alone.
pub(super) fn check_lazy_train_at_threshold<C: TraversalCodec>(dim: usize) {
    let hnsw = euclidean_backend::<C>(dim, 16, 100, 100);
    for v in &sinusoidal_vectors(100, dim) {
        hnsw.insert(v).expect("test");
    }
    assert!(
        hnsw.is_quantizer_trained(),
        "Quantizer should be trained after threshold"
    );
}

/// `force_train_quantizer` trains below the threshold.
pub(super) fn check_force_train<C: TraversalCodec>(dim: usize) {
    let hnsw = euclidean_backend::<C>(dim, 16, 100, 1000);
    insert_ramp(&hnsw, dim, 0..50);
    assert!(!hnsw.is_quantizer_trained());
    hnsw.force_train_quantizer().expect("test");
    assert!(hnsw.is_quantizer_trained());
}

/// After training, the quantized path returns results sorted ascending by
/// the transformed Euclidean distance.
pub(super) fn check_post_training_search_sorted<C: TraversalCodec>(dim: usize) {
    let hnsw = euclidean_backend::<C>(dim, 16, 100, 1000);
    for v in &sinusoidal_vectors(200, dim) {
        hnsw.insert(v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = hnsw.search_bounded(&query, 10, 50, C::DEFAULT_OVERSAMPLING, 0);
    assert!(!results.is_empty());
    for i in 1..results.len() {
        assert!(
            results[i].1 >= results[i - 1].1,
            "Results should be sorted by distance"
        );
    }
}

/// Inserts landing after training are encoded at their assigned node slot:
/// the post-training vector's self-query must surface its own node id.
pub(super) fn check_insert_after_training_alignment<C: TraversalCodec>(dim: usize) {
    let hnsw = euclidean_backend::<C>(dim, 16, 100, 1000);
    insert_ramp(&hnsw, dim, 0..50);
    hnsw.force_train_quantizer().expect("test");
    insert_ramp(&hnsw, dim, 50..100);
    assert_eq!(hnsw.len(), 100);

    let query: Vec<f32> = (0..dim).map(|j| (75 * dim + j) as f32).collect();
    let results = hnsw.search_bounded(&query, 5, 50, C::DEFAULT_OVERSAMPLING, 0);
    assert_eq!(
        results[0].0, 75,
        "top-1 must be node 75 (post-training insert whose vector exactly matches the query) — \
         a misaligned store slot would surface the wrong node"
    );
}

/// Installing a pre-trained quantizer must encode every existing vector
/// (store rebuilt in NodeId order) and activate quantized search with
/// recall of at least `min_recall` against the f32 baseline.
pub(super) fn check_install_encodes_existing<C: TraversalCodec>(
    dim: usize,
    train: fn(&[Vec<f32>]) -> Arc<C::Quantizer>,
    min_recall: f64,
) {
    let (n, k) = (200, 10);
    let hnsw = euclidean_backend::<C>(dim, 16, 200, 1000);
    let vectors = sinusoidal_vectors(n, dim);
    for v in &vectors {
        hnsw.insert(v).expect("insert");
    }
    assert!(!hnsw.is_quantizer_trained(), "below lazy-train threshold");

    let query = &vectors[42];
    let baseline: HashSet<usize> = hnsw
        .search(query, k, 100)
        .iter()
        .map(|&(id, _)| id)
        .collect();

    hnsw.install_trained_quantizer(train(&vectors))
        .expect("install");
    assert!(hnsw.is_quantizer_trained());

    let results = hnsw.search_bounded(query, k, 100, C::DEFAULT_OVERSAMPLING, 0);
    assert_eq!(results.len(), k);
    assert_eq!(results[0].0, 42, "self-query must return itself as top-1");

    let ids: HashSet<usize> = results.iter().map(|&(id, _)| id).collect();
    let overlap = baseline.intersection(&ids).count();
    #[allow(clippy::cast_precision_loss)]
    let recall = overlap as f64 / k as f64;
    assert!(
        recall >= min_recall,
        "quantized results should overlap f32 baseline (recall sanity), got {recall:.2}"
    );
}

/// Inserts after install must stay aligned with NodeId order: the store was
/// rebuilt for nodes `0..n`, so node `n` (first post-install insert) must be
/// encoded at store position `n` and remain searchable.
pub(super) fn check_install_then_insert_alignment<C: TraversalCodec>(
    dim: usize,
    train: fn(&[Vec<f32>]) -> Arc<C::Quantizer>,
) {
    let n = 120;
    let hnsw = euclidean_backend::<C>(dim, 16, 200, 1000);
    let vectors = sinusoidal_vectors(n + 30, dim);
    for v in &vectors[..n] {
        hnsw.insert(v).expect("insert");
    }
    hnsw.install_trained_quantizer(train(&vectors[..n]))
        .expect("install");
    for v in &vectors[n..] {
        hnsw.insert(v).expect("post-install insert");
    }
    assert_eq!(hnsw.len(), n + 30);

    // Self-query on a post-install vector: top-1 must be its own node id.
    let target = n + 15;
    let results = hnsw.search_bounded(&vectors[target], 5, 100, C::DEFAULT_OVERSAMPLING, 0);
    assert_eq!(
        results.first().map(|&(id, _)| id),
        Some(target),
        "post-install vector must be searchable at its node id"
    );
}

/// The rerank path must return actual Euclidean distances (with sqrt), NOT
/// the engine's raw squared L2 — `transform_score` applied end-to-end.
pub(super) fn check_euclidean_rerank_is_sqrt<C: TraversalCodec>(dim: usize) {
    let hnsw = euclidean_backend::<C>(dim, 16, 100, 1000);
    // v0 = origin, v1 = ones: Euclidean distance sqrt(dim); squared L2
    // would be dim as f32 — the historical bug value.
    hnsw.insert(&vec![0.0_f32; dim]).expect("test");
    hnsw.insert(&vec![1.0_f32; dim]).expect("test");
    hnsw.force_train_quantizer().expect("test");

    let results = hnsw.search_bounded(&vec![0.0_f32; dim], 2, 50, C::DEFAULT_OVERSAMPLING, 0);
    assert!(
        results.len() >= 2,
        "Expected at least 2 results, got {}",
        results.len()
    );
    let v1_dist = results
        .iter()
        .find(|(id, _)| *id == 1)
        .map(|(_, d)| *d)
        .expect("v1 should be in results");
    let expected = (dim as f32).sqrt();
    assert!(
        (v1_dist - expected).abs() < 0.01,
        "Distance to v1 should be sqrt({dim}) ~= {expected:.3}, got {v1_dist:.3} \
         (if ~{dim}.0, transform_score was not applied)"
    );
}

/// Below the codec's default `min_index_size`, a TRAINED index must skip
/// the quantized path and return exactly the pre-training f32 results —
/// the guard short-circuits before any codec machinery runs.
pub(super) fn check_below_min_index_fallback<C: TraversalCodec>(dim: usize) {
    let hnsw = euclidean_backend::<C>(dim, 16, 100, 1000);
    let vectors = sinusoidal_vectors(100, dim);
    for v in &vectors {
        hnsw.insert(v).expect("test");
    }
    let baseline = hnsw.search(&vectors[42], 10, 100);

    hnsw.force_train_quantizer().expect("test");
    assert!(hnsw.is_quantizer_trained());

    // 100 vectors < the codec's default min_index_size: default search must
    // produce the identical exact-f32 result list.
    let fallback = hnsw.search(&vectors[42], 10, 100);
    assert_eq!(
        fallback, baseline,
        "below-min search must stay on exact f32"
    );
}

/// Cosine traversal must survive an UNNORMALIZED query: codes are built
/// from the prepared (unit-norm) stored form and the query is prepared the
/// same way before any codec sees it, so scaling the query must not change
/// the result set. Pins the encode/prepare-space alignment for every codec
/// (`RaBitQ`'s centroid subtraction is not scale-invariant, so encoding raw
/// inputs would break exactly this property).
pub(super) fn check_cosine_scale_invariance<C: TraversalCodec>(dim: usize) {
    let k = 10;
    let (hnsw, vectors) = trained_planted_backend::<C>(DistanceMetric::Cosine, dim);
    let query = &vectors[PLANTED_QUERY_ID];

    let scaled: Vec<f32> = query.iter().map(|x| x * 37.5).collect();
    let from_unit = hnsw.search_bounded(query, k, 100, C::DEFAULT_OVERSAMPLING, 0);
    let from_scaled = hnsw.search_bounded(&scaled, k, 100, C::DEFAULT_OVERSAMPLING, 0);

    let unit_ids: Vec<usize> = from_unit.iter().map(|&(id, _)| id).collect();
    let scaled_ids: Vec<usize> = from_scaled.iter().map(|&(id, _)| id).collect();
    assert_eq!(
        unit_ids, scaled_ids,
        "cosine is scale-invariant: quantized traversal must not depend on query norm"
    );
}

/// Recall contract on the wired default path: recall@10 >= 0.95 on 10K
/// vectors through `search()` (quantizer auto-trained at 1000 inserts,
/// quantized traversal active at the codec's default activation size).
pub(super) fn check_recall_10k<C: TraversalCodec>() {
    let (dim, n, k, ef_search) = (128, 10_000, 10, 200);
    let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Euclidean, dim);
    let hnsw = Backend::<C>::new(engine, dim, 32, 200, n).expect("test");

    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();
    for v in &vectors {
        hnsw.insert(v).expect("test");
    }
    assert!(hnsw.is_quantizer_trained(), "auto-trained at 1000 inserts");

    let query_indices = [0, 1000, 5000, 7777, 9999];
    let mut total_recall = 0.0;
    for &qi in &query_indices {
        let query = &vectors[qi];
        let brute_ids: HashSet<usize> =
            brute_force_top_ids(&vectors, query, DistanceMetric::Euclidean, k)
                .into_iter()
                .collect();

        let results = hnsw.search(query, k, ef_search);
        let result_ids: HashSet<usize> = results.iter().map(|(id, _)| *id).collect();
        let overlap = brute_ids.intersection(&result_ids).count();
        #[allow(clippy::cast_precision_loss)]
        let recall = overlap as f64 / k as f64;
        total_recall += recall;
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / query_indices.len() as f64;
    assert!(
        avg_recall >= 0.95,
        "recall@{k} should be >= 0.95 through the default config, got {avg_recall:.3}"
    );
}

/// A BFS locality reorder must not desynchronize the codes from the graph.
///
/// `ANALYZE` calls `reorder_for_locality()`, which renumbers every node. The
/// codec's store is **positional** — entry N holds node N's code — so it has
/// to be rebuilt against the new numbering; if it is not, traversal scores
/// every candidate against another node's code while the re-rank quietly
/// reports honest distances for whatever that traversal happened to reach.
/// Measured before the fix: 1.000 to 0.000, no error raised.
///
/// Quality is compared by **score**, not by id: a reorder renumbers nodes by
/// design, so comparing raw node ids across one would fail even for a
/// correct index. What must survive is that the k best distances are still
/// the k best distances.
pub(super) fn check_recall_survives_reorder<C: TraversalCodec>() {
    let (dim, n, k, ef_search) = (64, 2_000, 10, 200);
    let hnsw = euclidean_backend::<C>(dim, 32, 200, n);
    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();
    for v in &vectors {
        hnsw.insert(v).expect("test");
    }
    assert!(hnsw.is_quantizer_trained(), "auto-trained at 1000 inserts");

    let queries = [0, 500, 1234, 1999];
    let before = mean_score_recall(&hnsw, &vectors, &queries, k, ef_search);
    assert!(
        before >= 0.95,
        "baseline recall must hold before the reorder, got {before:.3}"
    );

    // Through the wrapper, which is what the backend dispatch calls: the
    // inner graph cannot keep the store aligned on its own.
    hnsw.reorder_for_locality().expect("test");

    let after = mean_score_recall(&hnsw, &vectors, &queries, k, ef_search);
    assert!(
        after >= 0.95,
        "recall@{k} collapsed from {before:.3} to {after:.3} across a locality \
         reorder — the codes no longer describe the nodes they are indexed by"
    );
}

/// Mean fraction of a query's returned scores that are genuine top-k scores.
///
/// Brute force gives the k best distances; a search that reached the right
/// neighbourhood returns those same values whatever ids the nodes now carry.
#[allow(clippy::cast_precision_loss)]
fn mean_score_recall<C: TraversalCodec>(
    hnsw: &Backend<C>,
    vectors: &[Vec<f32>],
    queries: &[usize],
    k: usize,
    ef_search: usize,
) -> f64 {
    const TOLERANCE: f32 = 1e-4;
    let total: f64 = queries
        .iter()
        .map(|&qi| {
            let query = &vectors[qi];
            let best: Vec<f32> = brute_force_top_ids(vectors, query, DistanceMetric::Euclidean, k)
                .into_iter()
                .map(|i| DistanceMetric::Euclidean.calculate(query, &vectors[i]))
                .collect();
            // `search_bounded` with a zero floor, not `search`: the default
            // `min_index_size` (10 000) would route a 2 000-vector index to
            // the exact-f32 fallback, and a test that never reads a code
            // cannot notice the codes being wrong.
            let hit = hnsw
                .search_bounded(query, k, ef_search, C::DEFAULT_OVERSAMPLING, 0)
                .iter()
                .filter(|(_, score)| best.iter().any(|b| (b - score).abs() <= TOLERANCE))
                .count();
            hit as f64 / k as f64
        })
        .sum();
    total / queries.len() as f64
}
