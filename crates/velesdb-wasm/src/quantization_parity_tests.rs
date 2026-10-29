//! WASM ↔ core search parity per storage mode.
//!
//! The contract: a `VectorStore` created with a given storage mode returns the
//! same top-k, with the same scores, as a `velesdb-core` collection created
//! with that mode, on the same data. Core keeps the f32 vectors in every mode
//! and its quantized paths re-rank the final top-k with exact f32 distances
//! (`StorageMode`'s docs in `velesdb-core/src/quantization/mod.rs`), with
//! `Binary` behaving as `Full`. So the browser store keeps the f32 vectors for
//! every mode too and scores them exactly.
//!
//! Before this contract, the browser store kept only lossy codes for
//! `sq8`/`pq`/`rabitq` (a per-vector SQ8) and `binary` (sign bits decoded to
//! 0/1), and scored the reconstruction: the same mode name returned different
//! scores and a different order than core, and `get()` returned the
//! reconstruction instead of the inserted vector.
//!
//! The core side runs through a real `Database`/`VectorCollection`, which is
//! only compiled with core's `persistence` feature: the native-only
//! dev-dependency in `Cargo.toml` turns it on for host test builds, so this
//! module is excluded from `wasm32` test builds, where that dependency does
//! not exist.
//!
//! Core is compared through two entry points. `search_with_quality(Perfect)`
//! is core's exact search for every mode — the result its approximate HNSW
//! paths converge to — and runs here on enough points that core's lazy SQ8,
//! `RaBitQ` and PQ quantizers have trained. Default `search()` is compared on a
//! collection small enough for core to brute-force it (`<= 100` points), which
//! also keeps it below core's PQ training threshold: once PQ trains, core's
//! default `search()` returns ADC-approximated scores while
//! `search_with_quality` keeps exact ones, a split inside core this module
//! does not paper over.

use velesdb_core::{
    Database, Point, SearchQuality, SearchResult, StorageMode as CoreStorageMode, VectorCollection,
};

use crate::store_get::get_vector_at_index;
use crate::store_insert::insert_vector;
use crate::store_new::create_store;
use crate::{DistanceMetric, StorageMode, VectorStore};

const DIMENSION: usize = 16;
const TOP_K: usize = 10;
/// Absolute score tolerance: core's SIMD kernels (and its pre-normalised
/// cosine) sum in a different order than the scalar `DistanceMetric::calculate`
/// the browser store uses, so scores agree to rounding, not bit for bit.
const SCORE_TOLERANCE: f32 = 1.0e-4;

const MODES: [(StorageMode, CoreStorageMode); 5] = [
    (StorageMode::Full, CoreStorageMode::Full),
    (StorageMode::SQ8, CoreStorageMode::SQ8),
    (StorageMode::Binary, CoreStorageMode::Binary),
    (
        StorageMode::ProductQuantization,
        CoreStorageMode::ProductQuantization,
    ),
    (StorageMode::RaBitQ, CoreStorageMode::RaBitQ),
];

const METRICS: [DistanceMetric; 3] = [
    DistanceMetric::Cosine,
    DistanceMetric::Euclidean,
    DistanceMetric::DotProduct,
];

/// Deterministic vectors in `[-1, 1)` (xorshift64*), so a failure reproduces.
fn dataset(count: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut state = seed.max(1);
    let mut next = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let bits = state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        (bits as f32 / (1u64 << 24) as f32).mul_add(2.0, -1.0)
    };
    (0..count)
        .map(|_| (0..DIMENSION).map(|_| next()).collect())
        .collect()
}

fn wasm_store(vectors: &[Vec<f32>], metric: DistanceMetric, mode: StorageMode) -> VectorStore {
    let mut store = create_store(DIMENSION, metric, mode);
    for (id, vector) in (0u64..).zip(vectors) {
        insert_vector(&mut store, id, vector);
    }
    store
}

/// Runs `search` for each query against one core collection holding `vectors`.
fn core_results(
    vectors: &[Vec<f32>],
    metric: DistanceMetric,
    mode: CoreStorageMode,
    queries: &[Vec<f32>],
    search: impl Fn(&VectorCollection, &[f32]) -> Vec<SearchResult>,
) -> Vec<Vec<(u64, f32)>> {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open core database");
    db.create_vector_collection_with_options("parity", DIMENSION, metric, mode)
        .expect("create core collection");
    let collection = db.get_vector_collection("parity").expect("collection");
    let points: Vec<Point> = (0u64..)
        .zip(vectors)
        .map(|(id, v)| Point::without_payload(id, v.clone()))
        .collect();
    collection.upsert_bulk(&points).expect("core upsert");
    queries
        .iter()
        .map(|query| {
            search(&collection, query)
                .into_iter()
                .map(|r| (r.point.id, r.score))
                .collect()
        })
        .collect()
}

fn assert_same_top_k(label: &str, wasm: &[(u64, f32)], core: &[(u64, f32)]) {
    let wasm_ids: Vec<u64> = wasm.iter().map(|r| r.0).collect();
    let core_ids: Vec<u64> = core.iter().map(|r| r.0).collect();
    assert_eq!(wasm_ids, core_ids, "{label}: top-k ids differ");
    for ((id, w), (_, c)) in wasm.iter().zip(core) {
        assert!(
            (w - c).abs() <= SCORE_TOLERANCE,
            "{label}: score of id {id} differs: wasm={w} core={c}"
        );
    }
}

/// Asserts the browser store and core agree on every query, in `mode`/`metric`.
fn assert_parity(
    vectors: &[Vec<f32>],
    queries: &[Vec<f32>],
    metric: DistanceMetric,
    (mode, core_mode): (StorageMode, CoreStorageMode),
    search: impl Fn(&VectorCollection, &[f32]) -> Vec<SearchResult>,
) {
    let store = wasm_store(vectors, metric, mode);
    let core = core_results(vectors, metric, core_mode, queries, search);
    for (query, core) in queries.iter().zip(&core) {
        let wasm = store.search_scored(query, TOP_K);
        assert_same_top_k(&format!("{mode:?}/{metric:?}"), &wasm, core);
    }
}

/// Every mode, every metric, against core's exact search on a collection big
/// enough (1 200 points) for core's lazy SQ8/`RaBitQ` (1 000) and PQ (128)
/// quantizers to have trained.
#[test]
fn test_every_storage_mode_matches_core_exact_search() {
    let vectors = dataset(1_200, 0x5EED_0001);
    let queries = dataset(4, 0x5EED_0002);
    for metric in METRICS {
        for modes in MODES {
            assert_parity(&vectors, &queries, metric, modes, |c, q| {
                c.search_with_quality(q, TOP_K, SearchQuality::Perfect)
                    .expect("core search")
            });
        }
    }
}

/// Every mode against core's default `search()` entry point, on a collection
/// core brute-forces (`<= 100` points).
#[test]
fn test_every_storage_mode_matches_core_default_search() {
    let vectors = dataset(100, 0x5EED_0003);
    let queries = dataset(4, 0x5EED_0004);
    for metric in METRICS {
        for modes in MODES {
            assert_parity(&vectors, &queries, metric, modes, |c, q| {
                c.search(q, TOP_K).expect("core search")
            });
        }
    }
}

/// `get()` returns the inserted vector in every mode, as core does — not a
/// lossy reconstruction.
#[test]
fn test_every_storage_mode_returns_the_inserted_vector() {
    let vectors = dataset(3, 0x5EED_0005);
    for (mode, _) in MODES {
        let store = wasm_store(&vectors, DistanceMetric::Euclidean, mode);
        for (idx, vector) in vectors.iter().enumerate() {
            assert_eq!(
                &get_vector_at_index(&store, idx),
                vector,
                "{mode:?}: get() must return the inserted f32 vector"
            );
        }
    }
}
