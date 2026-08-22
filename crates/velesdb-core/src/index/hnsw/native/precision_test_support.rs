//! Shared fixtures and assertions for the quantized-precision backend tests
//! (`rabitq_precision_tests`, `sq8_precision_tests`).

use crate::distance::DistanceMetric;

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
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in &mut v {
        *x /= norm;
    }
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
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in &mut v {
            *x /= norm;
        }
        vectors[n - 10 + slot] = v;
    }
    vectors
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
    let got: std::collections::HashSet<usize> = results.iter().map(|(id, _)| *id).collect();
    let overlap = expected.iter().filter(|id| got.contains(id)).count();
    let recall = overlap as f64 / k as f64;
    assert!(
        recall >= 0.95,
        "recall@{k} vs brute-force must be >= 0.95 for {metric:?}, got {recall:.2}"
    );
}
