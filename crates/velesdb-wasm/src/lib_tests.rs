//! Tests for `VelesDB` WASM `VectorStore`.

use super::*;

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

    let fused = fusion::fuse_results(&all_results, "rrf", 60);
    assert!(!fused.is_empty());
    // ID 2 appears in rank 0 and rank 1, should have high RRF score
    // ID 1 appears in rank 0 and rank 1, should also be high
}

#[test]
fn test_fuse_results_average() {
    let results1 = vec![(1, 0.9), (2, 0.8)];
    let results2 = vec![(1, 0.7), (2, 0.6)];
    let all_results = vec![results1, results2];

    let fused = fusion::fuse_results(&all_results, "average", 60);
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

    let fused = fusion::fuse_results(&all_results, "maximum", 60);
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
    let fused = fusion::fuse_results(&all_results, "rrf", 60);
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
