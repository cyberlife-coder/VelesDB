use super::{rescore_euclidean_batch, PQVector, ProductQuantizer};
use crate::scored_result::ScoredResult;
use std::collections::HashMap;

fn small_trained_pq() -> ProductQuantizer {
    let vectors = vec![
        vec![1.0, 2.0, 3.0, 4.0],
        vec![5.0, 6.0, 7.0, 8.0],
        vec![-1.0, -2.0, 9.0, 10.0],
    ];
    ProductQuantizer::train(&vectors, 2, 2).expect("train small PQ")
}

#[test]
fn invalid_pq_code_in_search_path_skips_candidate_without_panic() {
    // Routing an out-of-range PQ code through the Euclidean batch scoring entry
    // point (the same one the fallback uses) must NOT panic and must NOT re-invoke
    // the unvalidated scalar indexing path. The candidate keeps its HNSW score.
    let quantizer = small_trained_pq();
    // num_centroids == 2, so code 99 is out of range for both subspaces.
    let bad = PQVector { codes: vec![0, 99] };

    let mut pq_cache: HashMap<u64, PQVector> = HashMap::new();
    pq_cache.insert(7, bad);

    let index_results = vec![ScoredResult::new(7, 0.42)];
    let query = vec![1.0, 2.0, 3.0, 4.0];

    let scored = rescore_euclidean_batch(&query, &quantizer, &pq_cache, &index_results);

    assert_eq!(scored.len(), 1);
    assert_eq!(scored[0].id, 7);
    // Clean skip: original HNSW score is retained, no panic, no garbage.
    assert!(
        (scored[0].score - 0.42).abs() < 1e-6,
        "rejected candidate must keep its HNSW score, got {}",
        scored[0].score
    );
}
