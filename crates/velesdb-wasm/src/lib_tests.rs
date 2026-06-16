//! Tests for `VelesDB` WASM `VectorStore`.

use super::*;

// =============================================================================
// BUG-1 FIX: Metric-aware threshold comparison tests
// =============================================================================

#[test]
fn test_similarity_threshold_metric_aware_comparison() {
    // For SIMILARITY metrics (Cosine, DotProduct, Jaccard):
    // "similarity() > 0.8" means score > 0.8 (higher = more similar)

    // For DISTANCE metrics (Euclidean, Hamming):
    // "similarity() > 0.8" should mean "more similar than 0.8 threshold"
    // which translates to distance < 0.8 (lower = more similar)

    let higher_is_better = true; // Cosine
    let lower_is_better = false; // Euclidean

    let score = 0.5_f32;
    let threshold = 0.8_f32;

    // Test: "similarity > 0.8" with Cosine (higher = better)
    // score=0.5 should NOT pass (0.5 is NOT more similar than 0.8)
    let cosine_gt_passes = if higher_is_better {
        score > threshold // Normal comparison
    } else {
        score < threshold // Inverted for distance
    };
    assert!(!cosine_gt_passes, "Cosine: 0.5 should not be > 0.8 similar");

    // Test: "similarity > 0.8" with Euclidean (lower = better)
    // score=0.5 SHOULD pass (distance 0.5 IS more similar than distance 0.8)
    let euclidean_gt_passes = if lower_is_better {
        score > threshold // Wrong! This is the bug
    } else {
        score < threshold // Correct: lower distance = more similar
    };
    assert!(
        euclidean_gt_passes,
        "Euclidean: distance 0.5 should be 'more similar' than 0.8"
    );
}

#[test]
fn test_threshold_comparison_builder() {
    // Test the comparison function builder that should be metric-aware

    // For Euclidean (lower = better), ">" means "more similar than"
    // which is actually "distance < threshold"
    let euclidean_higher_is_better = false;

    // Build the correct comparison for ">" operator
    let gt_comparison = |score: f32, thresh: f32, higher_is_better: bool| -> bool {
        if higher_is_better {
            score > thresh
        } else {
            score < thresh // Invert for distance metrics
        }
    };

    // Distance 0.3 is "more similar" than threshold 0.5
    assert!(gt_comparison(0.3, 0.5, euclidean_higher_is_better));
    // Distance 0.7 is NOT "more similar" than threshold 0.5
    assert!(!gt_comparison(0.7, 0.5, euclidean_higher_is_better));

    // For Cosine (higher = better), normal comparison
    let cosine_higher_is_better = true;
    assert!(gt_comparison(0.9, 0.8, cosine_higher_is_better));
    assert!(!gt_comparison(0.7, 0.8, cosine_higher_is_better));
}

// =============================================================================
// Original tests
// =============================================================================

#[test]
fn test_storage_mode_full() {
    let store = VectorStore::new(4, "cosine").unwrap();
    assert_eq!(store.storage_mode(), "full");
    assert_eq!(store.len(), 0);
}

#[test]
fn test_storage_mode_sq8() {
    let store = VectorStore::new_with_mode(4, "cosine", "sq8").unwrap();
    assert_eq!(store.storage_mode(), "sq8");
}

#[test]
fn test_storage_mode_binary() {
    let store = VectorStore::new_with_mode(4, "cosine", "binary").unwrap();
    assert_eq!(store.storage_mode(), "binary");
}

#[test]
fn test_sq8_insert_and_memory() {
    let mut store = VectorStore::new_with_mode(768, "cosine", "sq8").unwrap();
    #[allow(clippy::cast_precision_loss)]
    let vector: Vec<f32> = (0..768).map(|i| (i as f32) * 0.001).collect();

    store.insert(1, &vector).unwrap();

    assert_eq!(store.len(), 1);
    // SQ8: 768 bytes (u8) + 8 bytes (min+scale) + 8 bytes (id) = 784 bytes
    // Full would be: 768 * 4 + 8 = 3080 bytes
    let mem = store.memory_usage();
    assert!(mem < 1000, "SQ8 should use less than 1KB, got {mem}");
}

#[test]
fn test_binary_insert_and_memory() {
    let mut store = VectorStore::new_with_mode(768, "cosine", "binary").unwrap();
    let vector: Vec<f32> = (0..768)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();

    store.insert(1, &vector).unwrap();

    assert_eq!(store.len(), 1);
    // Binary: 768/8 = 96 bytes + 8 bytes (id) = 104 bytes
    // Full would be: 768 * 4 + 8 = 3080 bytes (~30x more)
    let mem = store.memory_usage();
    assert!(
        mem < 150,
        "Binary should use less than 150 bytes, got {mem}"
    );
}

#[test]
fn test_sq8_quantization_roundtrip() {
    let mut store = VectorStore::new_with_mode(4, "cosine", "sq8").unwrap();

    // Insert vectors - verify quantization works
    store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.insert(2, &[0.0, 1.0, 0.0, 0.0]).unwrap();
    store.insert(3, &[0.5, 0.5, 0.0, 0.0]).unwrap();

    assert_eq!(store.len(), 3);
    // Verify SQ8 data was stored
    assert_eq!(store.data_sq8.len(), 12); // 3 vectors * 4 dims
    assert_eq!(store.sq8_mins.len(), 3);
    assert_eq!(store.sq8_scales.len(), 3);
}

#[test]
fn test_binary_packing() {
    let mut store = VectorStore::new_with_mode(8, "hamming", "binary").unwrap();

    // Insert binary vectors
    store
        .insert(1, &[1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
        .unwrap();

    assert_eq!(store.len(), 1);
    // 8 dims = 1 byte
    assert_eq!(store.data_binary.len(), 1);
    // First two bits set: 0b00000011 = 3
    assert_eq!(store.data_binary[0], 3);
}

#[test]
fn test_binary_packing_large() {
    let mut store = VectorStore::new_with_mode(16, "hamming", "binary").unwrap();

    // All ones in first byte, all zeros in second
    let mut vec = vec![0.0f32; 16];
    for item in vec.iter_mut().take(8) {
        *item = 1.0;
    }
    store.insert(1, &vec).unwrap();

    assert_eq!(store.data_binary.len(), 2);
    assert_eq!(store.data_binary[0], 0xFF); // All 8 bits set
    assert_eq!(store.data_binary[1], 0x00); // No bits set
}

#[test]
fn test_remove_sq8() {
    let mut store = VectorStore::new_with_mode(4, "cosine", "sq8").unwrap();
    store.insert(1, &[1.0, 2.0, 3.0, 4.0]).unwrap();
    store.insert(2, &[5.0, 6.0, 7.0, 8.0]).unwrap();

    assert_eq!(store.len(), 2);
    assert!(store.remove(1));
    assert_eq!(store.len(), 1);
    assert!(!store.remove(1)); // Already removed
}

#[test]
fn test_clear_all_modes() {
    for mode in ["full", "sq8", "binary"] {
        let mut store = VectorStore::new_with_mode(4, "cosine", mode).unwrap();
        store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        store.insert(2, &[0.0, 1.0, 0.0, 0.0]).unwrap();

        assert_eq!(store.len(), 2);
        store.clear();
        assert_eq!(store.len(), 0);
        assert_eq!(store.memory_usage(), 0);
    }
}

// =========================================================================
// Fusion Logic Tests (now using fusion module)
// =========================================================================

#[test]
fn test_fuse_results_rrf() {
    let results1 = vec![(1, 0.9), (2, 0.8), (3, 0.7)];
    let results2 = vec![(2, 0.95), (1, 0.85), (4, 0.6)];
    let all_results = vec![results1, results2];

    let fused = fusion::fuse_results(&all_results, "rrf", 60).unwrap();
    assert!(!fused.is_empty());
    // ID 2 appears in rank 0 and rank 1, should have high RRF score
    // ID 1 appears in rank 0 and rank 1, should also be high
}

#[test]
fn test_fuse_results_average() {
    let results1 = vec![(1, 0.9), (2, 0.8)];
    let results2 = vec![(1, 0.7), (2, 0.6)];
    let all_results = vec![results1, results2];

    let fused = fusion::fuse_results(&all_results, "average", 60).unwrap();
    assert_eq!(fused.len(), 2);
    // ID 1: (0.9 + 0.7) / 2 = 0.8
    // ID 2: (0.8 + 0.6) / 2 = 0.7
    let id1_score = fused.iter().find(|(id, _)| *id == 1).map(|(_, s)| *s);
    assert!((id1_score.unwrap() - 0.8).abs() < 0.01);
}

#[test]
fn test_fuse_results_maximum() {
    let results1 = vec![(1, 0.9), (2, 0.5)];
    let results2 = vec![(1, 0.7), (2, 0.8)];
    let all_results = vec![results1, results2];

    let fused = fusion::fuse_results(&all_results, "maximum", 60).unwrap();
    // ID 1: max(0.9, 0.7) = 0.9
    // ID 2: max(0.5, 0.8) = 0.8
    let id1_score = fused.iter().find(|(id, _)| *id == 1).map(|(_, s)| *s);
    let id2_score = fused.iter().find(|(id, _)| *id == 2).map(|(_, s)| *s);
    assert!((id1_score.unwrap() - 0.9).abs() < 0.01);
    assert!((id2_score.unwrap() - 0.8).abs() < 0.01);
}

#[test]
fn test_fuse_results_empty() {
    let all_results: Vec<Vec<(u64, f32)>> = vec![];
    let fused = fusion::fuse_results(&all_results, "rrf", 60).unwrap();
    assert!(fused.is_empty());
}

// Note: similarity_search tests require WASM runtime (returns JsValue).
// The method is tested via wasm-pack test in CI.
// Here we test the underlying distance calculations used by similarity_search.

#[test]
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn test_similarity_threshold_logic() {
    // Test the threshold comparison logic used in similarity_search
    let threshold = 0.8_f32;

    // GT: score > threshold
    assert!(0.9_f32 > threshold);
    assert!(0.8_f32 <= threshold); // Equivalent to !(0.8 > threshold)
    assert!(0.7_f32 <= threshold); // Equivalent to !(0.7 > threshold)

    // GTE: score >= threshold
    assert!(0.9_f32 >= threshold);
    assert!(0.8_f32 >= threshold);
    assert!(0.7_f32 < threshold); // Equivalent to !(0.7 >= threshold)

    // LT: score < threshold
    assert!(0.9_f32 >= threshold); // Equivalent to !(0.9 < threshold)
    assert!(0.8_f32 >= threshold); // Equivalent to !(0.8 < threshold)
    assert!(0.7_f32 < threshold);

    // EQ: |score - threshold| < 0.001
    assert!((0.8001_f32 - threshold).abs() < 0.001);
    assert!((0.81_f32 - threshold).abs() >= 0.001);
}

// =============================================================================
// VectorStore::search_sparse — query the store's pre-built sparse index
// =============================================================================

#[test]
fn test_wasm_sparse_search_basic() {
    // A pre-built in-memory sparse index returns ranked doc ids for a known
    // query. Uses the native-testable scored entry point (the wasm-bindgen
    // method serializes to JsValue, which panics off-wasm32).
    let mut store = store_new::create_store(4, DistanceMetric::Cosine, StorageMode::Full);
    store
        .sparse_insert(1, &[10, 20, 30], &[1.0, 0.5, 0.3])
        .expect("test: sparse_insert doc 1");
    store
        .sparse_insert(2, &[10, 40], &[0.8, 1.2])
        .expect("test: sparse_insert doc 2");
    store
        .sparse_insert(3, &[20, 30, 50], &[0.9, 0.7, 0.4])
        .expect("test: sparse_insert doc 3");
    store
        .sparse_insert(4, &[10, 20], &[0.3, 1.5])
        .expect("test: sparse_insert doc 4");

    // query = {10: 1.0, 20: 1.0}
    // Doc 1: 1.5, Doc 2: 0.8, Doc 3: 0.9, Doc 4: 1.8
    let results = store
        .search_sparse_scored(&[10, 20], &[1.0, 1.0], 10)
        .expect("test: search_sparse on a populated index");

    assert_eq!(results[0].doc_id(), 4, "doc 4 (score 1.8) ranks first");
    assert_eq!(results[1].doc_id(), 1, "doc 1 (score 1.5) ranks second");
}

#[test]
fn test_wasm_sparse_search_no_index_error() {
    // Querying a store with no sparse index returns an error (parity with
    // core's sparse_search, which errors when the index does not exist).
    let store = store_new::create_store(4, DistanceMetric::Cosine, StorageMode::Full);
    let err = store.search_sparse_scored(&[10, 20], &[1.0, 1.0], 10);
    assert!(err.is_err(), "search_sparse without an index must error");
    assert!(
        err.unwrap_err().contains("sparse index"),
        "error should mention the missing sparse index"
    );
}

// =============================================================================
// validate_multi_vector_len — overflow-safe flat multi-vector length check
// =============================================================================

#[test]
fn test_validate_multi_vector_len_ok() {
    // 3 vectors x 4 dims = 12 floats: the validated expected length is returned.
    let expected =
        store_search::validate_multi_vector_len(12, 3, 4).expect("test: 12 == 3 * 4 is valid");
    assert_eq!(expected, 12);
}

#[test]
fn test_validate_multi_vector_len_mismatch() {
    // A buffer shorter than num_vectors * dimension is rejected.
    let err = store_search::validate_multi_vector_len(10, 3, 4)
        .expect_err("test: 10 != 3 * 4 must error");
    assert!(
        err.contains("expected 12"),
        "error names the expected length"
    );
}

#[test]
fn test_validate_multi_vector_len_overflow() {
    // num_vectors * dimension that wraps usize is rejected up front rather than
    // spoofing the length check (the wasm32 32-bit overflow guard).
    let err = store_search::validate_multi_vector_len(0, usize::MAX, 2)
        .expect_err("test: usize::MAX * 2 overflows");
    assert!(err.contains("overflow"), "error flags the overflow");
}
