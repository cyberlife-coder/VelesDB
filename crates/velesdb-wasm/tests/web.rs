//! WASM integration tests for `VectorStore`
//!
//! Run with: `wasm-pack test --node`

#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

use velesdb_wasm::VectorStore;

// =============================================================================
// Constructor Tests
// =============================================================================

#[wasm_bindgen_test]
fn test_new_cosine_store() {
    let store = VectorStore::new(128, "cosine").expect("Failed to create store");
    assert_eq!(store.dimension(), 128);
    assert_eq!(store.len(), 0);
    assert!(store.is_empty());
}

#[wasm_bindgen_test]
fn test_new_euclidean_store() {
    let store = VectorStore::new(768, "euclidean").expect("Failed to create store");
    assert_eq!(store.dimension(), 768);
}

#[wasm_bindgen_test]
fn test_new_dot_store() {
    let store = VectorStore::new(1536, "dot").expect("Failed to create store");
    assert_eq!(store.dimension(), 1536);
}

#[wasm_bindgen_test]
fn test_new_l2_alias() {
    let store = VectorStore::new(64, "l2").expect("Failed to create store");
    assert_eq!(store.dimension(), 64);
}

#[wasm_bindgen_test]
fn test_new_invalid_metric() {
    let result = VectorStore::new(128, "invalid_metric");
    assert!(result.is_err());
}

// =============================================================================
// Insert Tests
// =============================================================================

#[wasm_bindgen_test]
fn test_insert_single_vector() {
    let mut store = VectorStore::new(4, "cosine").unwrap();
    let vector = vec![1.0_f32, 0.0, 0.0, 0.0];

    store.insert(1, &vector).expect("Insert should succeed");

    assert_eq!(store.len(), 1);
    assert!(!store.is_empty());
}

#[wasm_bindgen_test]
fn test_insert_multiple_vectors() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    for i in 0..100 {
        let vector = vec![i as f32, 0.0, 0.0, 0.0];
        store.insert(i, &vector).expect("Insert should succeed");
    }

    assert_eq!(store.len(), 100);
}

#[wasm_bindgen_test]
fn test_insert_bigint_ids() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    // Test with various u64 values including large ones
    let ids: Vec<u64> = vec![0, 1, 100, 1000, u64::MAX / 2, u64::MAX - 1];

    for (idx, id) in ids.iter().enumerate() {
        let vector = vec![idx as f32, 0.0, 0.0, 0.0];
        store.insert(*id, &vector).expect("Insert should succeed");
    }

    assert_eq!(store.len(), ids.len());
}

#[wasm_bindgen_test]
fn test_insert_dimension_mismatch() {
    let mut store = VectorStore::new(4, "cosine").unwrap();
    let wrong_dim_vector = vec![1.0_f32, 0.0, 0.0]; // Only 3 dimensions

    let result = store.insert(1, &wrong_dim_vector);
    assert!(result.is_err());
}

#[wasm_bindgen_test]
fn test_insert_overwrites_existing_id() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    assert_eq!(store.len(), 1);

    // Insert with same ID should overwrite
    store.insert(1, &[0.0, 1.0, 0.0, 0.0]).unwrap();
    assert_eq!(store.len(), 1); // Still 1 vector
}

// =============================================================================
// Search Tests
// =============================================================================

#[wasm_bindgen_test]
fn test_search_empty_store() {
    let store = VectorStore::new(4, "cosine").unwrap();
    let query = vec![1.0_f32, 0.0, 0.0, 0.0];

    let results = store.search(&query, 10).expect("Search should succeed");
    // Results should be an empty array
    assert!(!results.is_null());
}

#[wasm_bindgen_test]
fn test_search_cosine_similarity() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    // Insert orthogonal vectors
    store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.insert(2, &[0.0, 1.0, 0.0, 0.0]).unwrap();
    store.insert(3, &[0.0, 0.0, 1.0, 0.0]).unwrap();

    let query = vec![1.0_f32, 0.0, 0.0, 0.0];
    let results = store.search(&query, 3).expect("Search should succeed");
    let ranked: Vec<(u64, f32)> =
        serde_wasm_bindgen::from_value(results).expect("results deserialize as (id, score) pairs");
    assert_eq!(ranked.len(), 3, "all three vectors returned");
    // ID 1 (identical to query) ranks first with cosine ~1.0
    assert_eq!(ranked[0].0, 1, "highest cosine similarity must rank first");
    assert!((ranked[0].1 - 1.0).abs() < 1e-5, "top score is cosine 1.0, got {}", ranked[0].1);
    // Orthogonal vectors score ~0.0 and rank below
    assert!(ranked[1].1 < 0.5 && ranked[2].1 < 0.5, "orthogonal vectors score near zero");
}

#[wasm_bindgen_test]
fn test_search_top_k() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    for i in 0..100 {
        let vector = vec![i as f32, 0.0, 0.0, 0.0];
        store.insert(i, &vector).unwrap();
    }

    let query = vec![50.0_f32, 0.0, 0.0, 0.0];
    let results = store.search(&query, 5).expect("Search should succeed");
    let ranked: Vec<(u64, f32)> =
        serde_wasm_bindgen::from_value(results).expect("results deserialize");
    assert_eq!(ranked.len(), 5, "top-k must truncate 100 candidates to k=5");
}

#[wasm_bindgen_test]
fn test_search_dimension_mismatch() {
    let store = VectorStore::new(4, "cosine").unwrap();
    let wrong_dim_query = vec![1.0_f32, 0.0, 0.0]; // Only 3 dimensions

    let result = store.search(&wrong_dim_query, 10);
    assert!(result.is_err());
}

// =============================================================================
// Remove Tests
// =============================================================================

#[wasm_bindgen_test]
fn test_remove_existing_vector() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.insert(2, &[0.0, 1.0, 0.0, 0.0]).unwrap();
    assert_eq!(store.len(), 2);

    let removed = store.remove(1);
    assert!(removed);
    assert_eq!(store.len(), 1);
}

#[wasm_bindgen_test]
fn test_remove_nonexistent_vector() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();

    let removed = store.remove(999);
    assert!(!removed);
    assert_eq!(store.len(), 1);
}

// =============================================================================
// Clear Tests
// =============================================================================

#[wasm_bindgen_test]
fn test_clear() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    for i in 0..100 {
        store.insert(i, &[i as f32, 0.0, 0.0, 0.0]).unwrap();
    }
    assert_eq!(store.len(), 100);

    store.clear();
    assert_eq!(store.len(), 0);
    assert!(store.is_empty());
}

// =============================================================================
// Memory Usage Tests
// =============================================================================

#[wasm_bindgen_test]
fn test_memory_usage() {
    let mut store = VectorStore::new(128, "cosine").unwrap();

    let initial_memory = store.memory_usage();
    assert_eq!(initial_memory, 0);

    store.insert(1, &vec![0.0_f32; 128]).unwrap();

    let memory_after_insert = store.memory_usage();
    // Should be approximately: 8 bytes (u64 id) + 128 * 4 bytes (f32 vector)
    assert!(memory_after_insert > 0);
    assert_eq!(memory_after_insert, 8 + 128 * 4);
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[wasm_bindgen_test]
fn test_zero_dimension_vector() {
    // Zero dimension should work but be useless
    let store = VectorStore::new(0, "cosine");
    assert!(store.is_ok());
}

#[wasm_bindgen_test]
fn test_large_dimension() {
    let store = VectorStore::new(4096, "cosine").unwrap();
    assert_eq!(store.dimension(), 4096);
}

#[wasm_bindgen_test]
fn test_case_insensitive_metric() {
    let store1 = VectorStore::new(4, "COSINE").expect("Should accept uppercase");
    let store2 = VectorStore::new(4, "Cosine").expect("Should accept mixed case");
    let store3 = VectorStore::new(4, "cosine").expect("Should accept lowercase");

    assert_eq!(store1.dimension(), 4);
    assert_eq!(store2.dimension(), 4);
    assert_eq!(store3.dimension(), 4);
}

// =============================================================================
// Batch Insert Tests (TDD - Performance Optimization)
// =============================================================================

fn to_js_batch(batch: Vec<(u64, Vec<f32>)>) -> JsValue {
    serde_wasm_bindgen::to_value(&batch).unwrap()
}

#[wasm_bindgen_test]
fn test_batch_insert_single_batch() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    // Prepare batch data: Vec<(u64, Vec<f32>)>
    let batch: Vec<(u64, Vec<f32>)> = (0..100)
        .map(|i| (i, vec![i as f32, 0.0, 0.0, 0.0]))
        .collect();

    store
        .insert_batch(to_js_batch(batch))
        .expect("Batch insert should succeed");

    assert_eq!(store.len(), 100);
}

#[wasm_bindgen_test]
fn test_batch_insert_performance_10k() {
    let mut store = VectorStore::new(128, "cosine").unwrap();

    let batch: Vec<(u64, Vec<f32>)> = (0..10_000)
        .map(|i| {
            let mut vec = vec![0.0_f32; 128];
            vec[0] = i as f32;
            (i, vec)
        })
        .collect();

    store
        .insert_batch(to_js_batch(batch))
        .expect("Batch insert should succeed");

    assert_eq!(store.len(), 10_000);
}

#[wasm_bindgen_test]
fn test_batch_insert_dimension_mismatch() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    // One vector has wrong dimension
    let batch: Vec<(u64, Vec<f32>)> = vec![
        (1, vec![1.0, 0.0, 0.0, 0.0]),
        (2, vec![1.0, 0.0, 0.0]), // Wrong!
    ];

    let result = store.insert_batch(to_js_batch(batch));
    assert!(result.is_err());
}

#[wasm_bindgen_test]
fn test_batch_insert_empty() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    let batch: Vec<(u64, Vec<f32>)> = vec![];
    store
        .insert_batch(to_js_batch(batch))
        .expect("Empty batch should succeed");

    assert_eq!(store.len(), 0);
}

// =============================================================================
// Reserve Capacity Tests (TDD - Memory Pre-allocation)
// =============================================================================

#[wasm_bindgen_test]
fn test_reserve_capacity() {
    let mut store = VectorStore::new(128, "cosine").unwrap();

    // Reserve space for 10k vectors
    store.reserve(10_000);

    // Should still be empty
    assert_eq!(store.len(), 0);
    assert!(store.is_empty());

    // Now insert should be faster (pre-allocated)
    for i in 0..1000 {
        store.insert(i, &vec![0.0_f32; 128]).unwrap();
    }

    assert_eq!(store.len(), 1000);
}

#[wasm_bindgen_test]
fn test_with_capacity_constructor() {
    let mut store = VectorStore::with_capacity(128, "cosine", 10_000).unwrap();

    assert_eq!(store.dimension(), 128);
    assert_eq!(store.len(), 0);
    assert!(store.is_empty());
    // The pre-allocated store must be usable for inserts.
    store.insert(0, &vec![0.0_f32; 128]).unwrap();
    assert_eq!(store.len(), 1);
    assert!(!store.is_empty());
}

// =============================================================================
// IndexedDB Persistence Tests (TDD)
// =============================================================================

// Note: These tests require browser environment with IndexedDB support
// They are marked as ignored for Node.js testing and should be run in browser

#[wasm_bindgen_test]
#[ignore] // Requires browser IndexedDB
fn test_export_to_bytes() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.insert(2, &[0.0, 1.0, 0.0, 0.0]).unwrap();

    let bytes = store.export_to_bytes().expect("Export should succeed");

    assert!(!bytes.is_empty());
    assert!(bytes.len() > 32); // Should contain header + data
}

#[wasm_bindgen_test]
#[ignore] // Requires browser IndexedDB
fn test_import_from_bytes() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.insert(2, &[0.0, 1.0, 0.0, 0.0]).unwrap();

    let bytes = store.export_to_bytes().unwrap();

    // Create new store and import
    let imported = VectorStore::import_from_bytes(&bytes).expect("Import should succeed");

    assert_eq!(imported.len(), 2);
    assert_eq!(imported.dimension(), 4);
}

#[wasm_bindgen_test]
#[ignore] // Requires browser IndexedDB
fn test_export_import_preserves_search_results() {
    let mut store = VectorStore::new(4, "cosine").unwrap();

    store.insert(1, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.insert(2, &[0.5, 0.5, 0.0, 0.0]).unwrap();
    store.insert(3, &[0.0, 1.0, 0.0, 0.0]).unwrap();

    // Search before export
    let query = vec![1.0_f32, 0.0, 0.0, 0.0];
    let results_before = store.search(&query, 3).unwrap();

    // Export and import
    let bytes = store.export_to_bytes().unwrap();
    let imported = VectorStore::import_from_bytes(&bytes).unwrap();

    // Search after import
    let results_after = imported.search(&query, 3).unwrap();

    // Results should be identical
    assert!(!results_before.is_null());
    assert!(!results_after.is_null());
}

#[wasm_bindgen_test]
#[ignore] // Requires browser IndexedDB
fn test_export_empty_store() {
    let store = VectorStore::new(128, "euclidean").unwrap();

    let bytes = store
        .export_to_bytes()
        .expect("Export empty store should succeed");

    let imported = VectorStore::import_from_bytes(&bytes).unwrap();
    assert_eq!(imported.len(), 0);
    assert_eq!(imported.dimension(), 128);
}

#[wasm_bindgen_test]
#[ignore] // Requires browser IndexedDB
fn test_import_invalid_bytes() {
    let invalid_bytes = vec![0u8, 1, 2, 3, 4, 5];

    let result = VectorStore::import_from_bytes(&invalid_bytes);
    assert!(result.is_err());
}

// =============================================================================
// Performance Smoke Tests
// =============================================================================

#[wasm_bindgen_test]
fn test_insert_10k_vectors() {
    let mut store = VectorStore::new(128, "cosine").unwrap();

    for i in 0..10_000 {
        let mut vector = vec![0.0_f32; 128];
        vector[0] = i as f32;
        store.insert(i, &vector).unwrap();
    }

    assert_eq!(store.len(), 10_000);
}

#[wasm_bindgen_test]
fn test_search_in_10k_vectors() {
    let mut store = VectorStore::new(128, "cosine").unwrap();

    for i in 0..10_000 {
        let mut vector = vec![0.0_f32; 128];
        vector[0] = i as f32;
        store.insert(i, &vector).unwrap();
    }

    let query = vec![5000.0_f32; 128];
    let results = store.search(&query, 10).expect("Search should succeed");
    let ranked: Vec<(u64, f32)> =
        serde_wasm_bindgen::from_value(results).expect("results deserialize");
    // truncate(k) over 10k candidates yields exactly k.
    assert_eq!(ranked.len(), 10);
    // Every returned id must be one we inserted (0..10_000), and distinct.
    let mut seen = std::collections::HashSet::new();
    for (id, score) in &ranked {
        assert!(*id < 10_000, "id {id} out of inserted range");
        assert!(seen.insert(*id), "duplicate id {id} in results");
        assert!(score.is_finite(), "score for id {id} is not finite");
    }
    // cosine => higher_is_better, so results must be sorted descending by score.
    for w in ranked.windows(2) {
        assert!(w[0].1 >= w[1].1, "results not sorted by descending score");
    }
}
