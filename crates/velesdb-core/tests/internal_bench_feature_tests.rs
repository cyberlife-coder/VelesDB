#![cfg(feature = "internal-bench")]

use velesdb_core::internal_bench;
use velesdb_core::simd_native::{cosine_similarity_native, DistanceEngine};
use velesdb_core::sparse_index::{sparse_search, SparseInvertedIndex, SparseVector};

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

fn make_sparse_vector(pairs: &[(u32, f32)]) -> SparseVector {
    SparseVector::new(pairs.to_vec())
}

#[test]
fn test_internal_bench_cosine_wrappers_match_public_paths() {
    for dim in [1_usize, 8, 64, 768, 1536] {
        let (a, b) = sample_vectors(dim);
        let dispatch = cosine_similarity_native(&a, &b);
        let scalar = internal_bench::cosine_scalar(&a, &b);
        let resolved = internal_bench::cosine_resolved(&a, &b);
        let engine = DistanceEngine::new(dim);
        let public_resolved = engine.cosine_similarity(&a, &b);
        assert!((dispatch - scalar).abs() <= 1e-5, "dim={dim}");
        assert!((resolved - public_resolved).abs() <= 1e-5, "dim={dim}");
    }
}

#[test]
fn test_internal_bench_sparse_batch_matches_sequential_search() {
    let sequential = SparseInvertedIndex::new();
    let batched = SparseInvertedIndex::new();
    let docs = vec![
        (1_u64, make_sparse_vector(&[(1, 0.5), (3, 1.0), (7, 0.3)])),
        (2_u64, make_sparse_vector(&[(1, 0.8), (4, 0.5)])),
        (3_u64, make_sparse_vector(&[(2, 1.1), (7, 0.9)])),
        (4_u64, make_sparse_vector(&[(1, 1.2), (2, 0.2), (8, 0.4)])),
        (4_u64, make_sparse_vector(&[(1, 1.4), (2, 0.7), (8, 0.4)])),
    ];
    let query = make_sparse_vector(&[(1, 1.0), (7, 0.8)]);

    for (doc_id, vector) in &docs {
        sequential.insert(*doc_id, vector);
    }
    internal_bench::sparse_insert_batch(&batched, &docs);

    assert_eq!(batched.doc_count(), sequential.doc_count());
    assert_eq!(
        sparse_search(&batched, &query, 3),
        sparse_search(&sequential, &query, 3)
    );
}
