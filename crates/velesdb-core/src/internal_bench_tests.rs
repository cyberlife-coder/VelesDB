use crate::internal_bench;
use crate::simd_native::{cosine_similarity_native, DistanceEngine};

fn sample_vectors(dim: usize) -> (Vec<f32>, Vec<f32>) {
    let a = (0..dim)
        .map(|i| {
            let idx = f32::from(u16::try_from(i).expect("test dimensions fit in u16"));
            (idx * 0.13).sin()
        })
        .collect();
    let b = (0..dim)
        .map(|i| {
            let idx = f32::from(u16::try_from(i).expect("test dimensions fit in u16"));
            (idx * 0.17 + 1.0).cos()
        })
        .collect();
    (a, b)
}

#[test]
fn test_internal_bench_cosine_paths_match_public_dispatch() {
    for dim in [0_usize, 1, 7, 8, 63, 64, 127, 128, 767, 768, 769, 1536] {
        let (a, b) = sample_vectors(dim);
        let dispatch = cosine_similarity_native(&a, &b);
        let scalar = internal_bench::cosine_scalar(&a, &b);
        let resolved = internal_bench::cosine_resolved(&a, &b);
        assert!((dispatch - scalar).abs() <= 1e-5, "dim={dim}");
        assert!((dispatch - resolved).abs() <= 1e-5, "dim={dim}");
    }
}

#[test]
fn test_internal_bench_distance_engine_matches_public_engine() {
    for dim in [64_usize, 768, 1536] {
        let (a, b) = sample_vectors(dim);
        let public_engine = DistanceEngine::new(dim);
        let internal = internal_bench::cosine_resolved(&a, &b);
        let public = public_engine.cosine_similarity(&a, &b);
        assert!((internal - public).abs() <= 1e-5, "dim={dim}");
    }
}

// ===========================================================================
// #2177's measurement seam, covered where `cargo mutants` can see it.
//
// `sparse_linear_scan_search` and `sparse_maxscore_search` were exercised only
// by `tests/sparse_strategy_crossover.rs`. The mutants job runs
// `cargo mutants -- --lib --`, so an integration test is invisible to it and
// both seams' mutants survived: `replace … with vec![]` and
// `with vec![Default::default()]` were reported MISSED on a real run. A seam
// nothing pins can be emptied without a test noticing — and a seam that
// returns nothing would make the crossover harness compare two empty results
// and call them equal.
//
// These live here rather than beside the harness precisely because `--lib` is
// what the mutation job reads.
// ===========================================================================

/// Three documents whose term overlap is known, so the expected top-1 is not
/// a matter of opinion.
fn tiny_sparse_index() -> crate::index::sparse::SparseInvertedIndex {
    use crate::index::sparse::{SparseInvertedIndex, SparseVector};
    let index = SparseInvertedIndex::new();
    index.insert_batch_chunk(&[
        (1, SparseVector::new(vec![(1, 1.0), (2, 1.0)])),
        (2, SparseVector::new(vec![(2, 2.0), (3, 1.0)])),
        (3, SparseVector::new(vec![(4, 5.0)])),
    ]);
    index
}

#[test]
fn both_sparse_seams_return_the_documents_the_query_matches() {
    use crate::index::sparse::SparseVector;
    let index = tiny_sparse_index();
    let query = SparseVector::new(vec![(2, 1.0)]);

    for (name, hits) in [
        (
            "linear",
            internal_bench::sparse_linear_scan_search(&index, &query, 10),
        ),
        (
            "maxscore",
            internal_bench::sparse_maxscore_search(&index, &query, 10),
        ),
    ] {
        // `vec![]` and `vec![Default::default()]` both die here: the first on
        // the count, the second on the ids, which a default `ScoredDoc` cannot
        // carry.
        assert_eq!(hits.len(), 2, "{name}: term 2 is in documents 1 and 2");
        let ids: Vec<u64> = hits.iter().map(|h| h.doc_id).collect();
        assert!(ids.contains(&1) && ids.contains(&2), "{name}: got {ids:?}");
        assert!(
            !ids.contains(&3),
            "{name}: document 3 shares no term with the query"
        );
        // Document 2 weighs 2.0 against document 1's 1.0 on the only query
        // term, so the order is a fact about the data, not about the strategy.
        assert_eq!(hits[0].doc_id, 2, "{name}: highest weight ranks first");
        assert!(hits[0].score > hits[1].score, "{name}: scores are distinct");
    }
}

#[test]
fn the_sparse_scoring_counter_moves_and_resets() {
    use crate::index::sparse::SparseVector;
    let index = tiny_sparse_index();
    let query = SparseVector::new(vec![(2, 1.0)]);

    internal_bench::reset_sparse_scoring_ops();
    assert_eq!(internal_bench::sparse_scoring_ops(), 0, "reset means zero");
    let _ = internal_bench::sparse_linear_scan_search(&index, &query, 10);
    let after = internal_bench::sparse_scoring_ops();
    assert!(
        after > 0,
        "a scan that scored two documents counted no work"
    );

    internal_bench::reset_sparse_scoring_ops();
    assert_eq!(
        internal_bench::sparse_scoring_ops(),
        0,
        "the counter did not reset, so a second measurement would carry the first"
    );
}
