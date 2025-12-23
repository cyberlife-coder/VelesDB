//! `IndexedDB` Persistence Benchmarks
//!
//! Run with browser via wasm-pack test:
//! `wasm-pack test --headless --chrome -- --ignored`

#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

use velesdb_wasm::VectorStore;

wasm_bindgen_test_configure!(run_in_browser);

// Helper to convert batch to JsValue
fn to_js_batch(batch: Vec<(u64, Vec<f32>)>) -> JsValue {
    serde_wasm_bindgen::to_value(&batch).unwrap()
}

// =============================================================================
// Benchmark: Save to IndexedDB
// =============================================================================

#[wasm_bindgen_test]
#[ignore]
async fn bench_save_1k_768d() {
    let mut store = VectorStore::with_capacity(768, "cosine", 1_000).unwrap();

    // Insert 1k vectors
    let batch: Vec<(u64, Vec<f32>)> = (0..1_000_u64)
        .map(|i| {
            let vector: Vec<f32> = (0..768).map(|j| (i + j) as f32 * 0.001).collect();
            (i, vector)
        })
        .collect();
    store.insert_batch(to_js_batch(batch)).unwrap();

    // Benchmark save
    let start = js_sys::Date::now();
    store.save("bench_save_1k").await.unwrap();
    let elapsed = js_sys::Date::now() - start;

    web_sys::console::log_1(
        &format!(
            "📊 BENCH save_1k_768d: {:.2}ms ({:.2} MB/s)",
            elapsed,
            (1_000.0 * 768.0 * 4.0) / (elapsed * 1000.0) // MB/s
        )
        .into(),
    );

    // Cleanup
    VectorStore::delete_db("bench_save_1k").await.ok();

    assert!(elapsed < 5000.0, "Save 1k vectors should be < 5s");
}

#[wasm_bindgen_test]
#[ignore]
async fn bench_save_10k_768d() {
    let mut store = VectorStore::with_capacity(768, "cosine", 10_000).unwrap();

    // Insert 10k vectors
    let batch: Vec<(u64, Vec<f32>)> = (0..10_000_u64)
        .map(|i| {
            let vector: Vec<f32> = (0..768).map(|j| (i + j) as f32 * 0.001).collect();
            (i, vector)
        })
        .collect();
    store.insert_batch(to_js_batch(batch)).unwrap();

    // Benchmark save
    let start = js_sys::Date::now();
    store.save("bench_save_10k").await.unwrap();
    let elapsed = js_sys::Date::now() - start;

    let size_mb = (10_000.0 * 768.0 * 4.0) / (1024.0 * 1024.0);
    web_sys::console::log_1(
        &format!(
            "📊 BENCH save_10k_768d: {:.2}ms ({:.2} MB, {:.2} MB/s)",
            elapsed,
            size_mb,
            size_mb / (elapsed / 1000.0)
        )
        .into(),
    );

    // Cleanup
    VectorStore::delete_db("bench_save_10k").await.ok();

    assert!(elapsed < 30000.0, "Save 10k vectors should be < 30s");
}

// =============================================================================
// Benchmark: Load from IndexedDB
// =============================================================================

#[wasm_bindgen_test]
#[ignore]
async fn bench_load_1k_768d() {
    let mut store = VectorStore::with_capacity(768, "cosine", 1_000).unwrap();

    // Setup: insert and save
    let batch: Vec<(u64, Vec<f32>)> = (0..1_000_u64)
        .map(|i| {
            let vector: Vec<f32> = (0..768).map(|j| (i + j) as f32 * 0.001).collect();
            (i, vector)
        })
        .collect();
    store.insert_batch(to_js_batch(batch)).unwrap();
    store.save("bench_load_1k").await.unwrap();

    // Benchmark load
    let start = js_sys::Date::now();
    let loaded = VectorStore::load("bench_load_1k").await.unwrap();
    let elapsed = js_sys::Date::now() - start;

    web_sys::console::log_1(
        &format!(
            "📊 BENCH load_1k_768d: {:.2}ms ({} vectors loaded)",
            elapsed,
            loaded.len()
        )
        .into(),
    );

    // Cleanup
    VectorStore::delete_db("bench_load_1k").await.ok();

    assert_eq!(loaded.len(), 1_000);
    assert!(elapsed < 5000.0, "Load 1k vectors should be < 5s");
}

#[wasm_bindgen_test]
#[ignore]
async fn bench_load_10k_768d() {
    let mut store = VectorStore::with_capacity(768, "cosine", 10_000).unwrap();

    // Setup: insert and save
    let batch: Vec<(u64, Vec<f32>)> = (0..10_000_u64)
        .map(|i| {
            let vector: Vec<f32> = (0..768).map(|j| (i + j) as f32 * 0.001).collect();
            (i, vector)
        })
        .collect();
    store.insert_batch(to_js_batch(batch)).unwrap();
    store.save("bench_load_10k").await.unwrap();

    // Benchmark load
    let start = js_sys::Date::now();
    let loaded = VectorStore::load("bench_load_10k").await.unwrap();
    let elapsed = js_sys::Date::now() - start;

    let size_mb = (10_000.0 * 768.0 * 4.0) / (1024.0 * 1024.0);
    web_sys::console::log_1(
        &format!(
            "📊 BENCH load_10k_768d: {:.2}ms ({:.2} MB, {:.2} MB/s)",
            elapsed,
            size_mb,
            size_mb / (elapsed / 1000.0)
        )
        .into(),
    );

    // Cleanup
    VectorStore::delete_db("bench_load_10k").await.ok();

    assert_eq!(loaded.len(), 10_000);
    assert!(elapsed < 30000.0, "Load 10k vectors should be < 30s");
}

// =============================================================================
// Benchmark: Round-trip (Save + Load)
// =============================================================================

#[wasm_bindgen_test]
#[ignore]
async fn bench_roundtrip_5k_768d() {
    let mut store = VectorStore::with_capacity(768, "cosine", 5_000).unwrap();

    // Insert 5k vectors
    let batch: Vec<(u64, Vec<f32>)> = (0..5_000_u64)
        .map(|i| {
            let vector: Vec<f32> = (0..768).map(|j| (i + j) as f32 * 0.001).collect();
            (i, vector)
        })
        .collect();
    store.insert_batch(to_js_batch(batch)).unwrap();

    // Benchmark full round-trip
    let start = js_sys::Date::now();
    store.save("bench_roundtrip").await.unwrap();
    let loaded = VectorStore::load("bench_roundtrip").await.unwrap();
    let elapsed = js_sys::Date::now() - start;

    web_sys::console::log_1(
        &format!(
            "📊 BENCH roundtrip_5k_768d: {:.2}ms total ({} vectors)",
            elapsed,
            loaded.len()
        )
        .into(),
    );

    // Cleanup
    VectorStore::delete_db("bench_roundtrip").await.ok();

    assert_eq!(loaded.len(), 5_000);
    assert!(elapsed < 20000.0, "Round-trip 5k vectors should be < 20s");
}
