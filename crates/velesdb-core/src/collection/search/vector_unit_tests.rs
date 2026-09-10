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

/// An index search that fails must reach the caller as an error — never as an
/// empty answer, the one failure a caller cannot tell from a correct result
/// (#2246, P5).
///
/// A dimension mismatch is the error every index path can raise. The public
/// entry points check the dimension before they get here, which is why the old
/// swallow went unnoticed: nothing upstream could trigger it, so nothing saw it
/// would have read as "no match".
#[test]
fn an_index_search_error_is_returned_not_read_as_no_match() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let collection = crate::collection::types::Collection::create(
        dir.path().to_path_buf(),
        4,
        crate::DistanceMetric::Cosine,
    )
    .expect("test: create collection");
    collection
        .upsert(vec![crate::Point::without_payload(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
        )])
        .expect("test: upsert");

    let control = collection
        .search_ids_with_adc_if_pq(&[1.0, 0.0, 0.0, 0.0], 1, crate::SearchQuality::Balanced)
        .expect("CONTROL: a well-formed query succeeds");
    assert_eq!(control.len(), 1, "CONTROL: the fixture's point is found");

    let outcome =
        collection.search_ids_with_adc_if_pq(&[1.0, 0.0], 1, crate::SearchQuality::Balanced);
    assert!(
        outcome.is_err(),
        "a query the index refuses must be an error, got {outcome:?}"
    );
}
