//! PQ recall accuracy benchmark suite (5K vectors, 128d, uniform random).
//!
//! Measures recall@10 for PQ, OPQ, and `RaBitQ` quantization methods
//! against brute-force exact L2 search ground truth on a 5K-vector
//! uniform random dataset.
//!
//! Uses explicit `TRAIN QUANTIZER ON ... WITH (m=8, k=256)` via `Database`
//! + `VelesQL` instead of auto-training, ensuring controlled m/k parameters.
//!
//! Uniform random data in high dimensions produces well-separated nearest
//! neighbors, enabling HNSW (M=24, `ef_construction=300`, `ef_search=128`) to
//! achieve recall@10 above 0.92, satisfying PQ-07.

#![allow(clippy::cast_precision_loss)]

use std::collections::{HashMap, HashSet};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tempfile::{tempdir, TempDir};
use velesdb_core::velesql::Parser;
use velesdb_core::{Collection, Database, DistanceMetric, Point, StorageMode};

const DIMENSION: usize = 128;
const NUM_VECTORS: usize = 5_000;
const NUM_QUERIES: usize = 50;
const K: usize = 10;

/// Generate synthetic data with seeded RNG for reproducibility.
///
/// Uses uniform random vectors in `[-1, 1]^dim`. Uniform random data in high
/// dimensions produces well-separated nearest neighbors, which is ideal for
/// benchmarking HNSW recall without hitting distance-tie degeneracies that
/// occur with tightly clustered synthetic data.
fn generate_random_data(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| (0..dim).map(|_| rng.gen_range(-1.0_f32..1.0)).collect())
        .collect()
}

/// Compute L2 (squared Euclidean) distance between two vectors.
fn l2_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

/// Brute-force exact top-k search returning IDs sorted by distance.
fn brute_force_topk(query: &[f32], dataset: &[Vec<f32>], k: usize) -> Vec<u64> {
    let mut dists: Vec<(u64, f32)> = dataset
        .iter()
        .enumerate()
        .map(|(i, v)| {
            #[allow(clippy::cast_possible_truncation)]
            let id = i as u64;
            (id, l2_distance(query, v))
        })
        .collect();
    dists.sort_by(|a, b| a.1.total_cmp(&b.1));
    dists.iter().take(k).map(|(id, _)| *id).collect()
}

/// Compute recall@k: fraction of ground truth IDs present in results.
fn recall_at_k(ground_truth: &[u64], results: &[u64], k: usize) -> f64 {
    assert!(k > 0, "recall@k requires k > 0");
    let gt_set: HashSet<u64> = ground_truth.iter().take(k).copied().collect();
    let result_set: HashSet<u64> = results.iter().take(k).copied().collect();
    let intersection = gt_set.intersection(&result_set).count();
    #[allow(clippy::cast_precision_loss)]
    let recall = intersection as f64 / k as f64;
    recall
}

/// Build a collection with full-precision storage mode (no quantization).
///
/// Returns the `TempDir` alongside the `Collection` so the directory stays
/// alive for the benchmark duration and is cleaned up on drop.
fn build_pq_collection(
    storage_mode: StorageMode,
    dataset: &[Vec<f32>],
    dimension: usize,
    name: &str,
) -> (Collection, TempDir) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join(name);

    let collection =
        Collection::create_with_options(path, dimension, DistanceMetric::Euclidean, storage_mode)
            .expect("create collection");

    let points: Vec<Point> = dataset
        .iter()
        .enumerate()
        .map(|(id, v)| {
            #[allow(clippy::cast_possible_truncation)]
            let pid = id as u64;
            Point::new(pid, v.clone(), Some(serde_json::json!({})))
        })
        .collect();
    collection.upsert(points).expect("upsert");
    (collection, dir)
}

/// Build a collection via `Database` + `VelesQL` `TRAIN QUANTIZER` for explicit
/// PQ/OPQ/RaBitQ training with controlled parameters.
///
/// Returns `(Collection, Database, TempDir)` -- `Database` and `TempDir`
/// must stay alive to keep the collection valid.
fn build_trained_collection(
    dataset: &[Vec<f32>],
    dimension: usize,
    name: &str,
    train_query: &str,
) -> (Collection, Database, TempDir) {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open database");
    db.create_collection(name, dimension, DistanceMetric::Euclidean)
        .expect("create collection");

    let coll = db.get_collection(name).expect("get collection");
    let points: Vec<Point> = dataset
        .iter()
        .enumerate()
        .map(|(id, v)| {
            #[allow(clippy::cast_possible_truncation)]
            let pid = id as u64;
            Point::new(pid, v.clone(), Some(serde_json::json!({})))
        })
        .collect();
    coll.upsert(points).expect("upsert");

    // Train quantizer via VelesQL
    let query = Parser::parse(train_query).expect("parse TRAIN QUANTIZER");
    let params: HashMap<String, serde_json::Value> = HashMap::new();
    db.execute_query(&query, &params)
        .expect("execute TRAIN QUANTIZER");

    // Re-fetch collection after training
    let coll = db.get_collection(name).expect("get trained collection");
    (coll, db, dir)
}

/// Measure average recall@k for a collection against brute-force ground truth.
fn measure_recall(
    collection: &Collection,
    queries: &[Vec<f32>],
    dataset: &[Vec<f32>],
    k: usize,
) -> f64 {
    assert!(
        !queries.is_empty(),
        "measure_recall requires non-empty queries"
    );
    let mut total_recall = 0.0;
    for query in queries {
        let gt = brute_force_topk(query, dataset, k);
        let results = collection.search_ids(query, k).expect("search");
        let result_ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
        total_recall += recall_at_k(&gt, &result_ids, k);
    }
    #[allow(clippy::cast_precision_loss)]
    let avg = total_recall / queries.len() as f64;
    avg
}

#[allow(clippy::too_many_lines)]
fn pq_recall_benchmarks(c: &mut Criterion) {
    // Generate shared dataset and queries (uniform random for well-separated neighbors)
    let dataset = generate_random_data(NUM_VECTORS, DIMENSION, 42);
    let queries = generate_random_data(NUM_QUERIES, DIMENSION, 123);

    // --- Variant 1: PQ m=8 k=256 with default rescore (oversampling=4) ---
    let (pq_coll, _pq_db, _pq_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "pq_m8",
        "TRAIN QUANTIZER ON pq_m8 WITH (m=8, k=256, type='pq')",
    );
    let pq_recall = measure_recall(&pq_coll, &queries, &dataset, K);
    println!("PQ m=8 k=256 rescore recall@{K}: {pq_recall:.4}");

    // --- Variant 2: Full precision baseline ---
    let (full_collection, _full_dir) =
        build_pq_collection(StorageMode::Full, &dataset, DIMENSION, "full_ref");
    let full_recall = measure_recall(&full_collection, &queries, &dataset, K);
    println!("Full-precision recall@{K}: {full_recall:.4}");

    // --- Variant 3: PQ m=8 k=256 no rescore ---
    let (norescore_coll, _norescore_db, _norescore_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "pq_norescore",
        "TRAIN QUANTIZER ON pq_norescore WITH (m=8, k=256, type='pq', oversampling=0)",
    );
    let norescore_recall = measure_recall(&norescore_coll, &queries, &dataset, K);
    println!("PQ no-rescore recall@{K}: {norescore_recall:.4}");

    // --- Variant 4: OPQ m=8 k=256 with rescore ---
    let (opq_coll, _opq_db, _opq_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "opq_m8",
        "TRAIN QUANTIZER ON opq_m8 WITH (m=8, k=256, type='opq')",
    );
    let opq_recall = measure_recall(&opq_coll, &queries, &dataset, K);
    println!("OPQ m=8 k=256 rescore recall@{K}: {opq_recall:.4}");

    // --- Variant 5: RaBitQ 128d ---
    let (rabitq_coll, _rabitq_db, _rabitq_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "rabitq_128",
        "TRAIN QUANTIZER ON rabitq_128 WITH (m=8, type='rabitq')",
    );
    let rabitq_recall = measure_recall(&rabitq_coll, &queries, &dataset, K);
    println!("RaBitQ 128d recall@{K}: {rabitq_recall:.4}");

    // --- Variant 6: PQ m=8 k=256 with oversampling=8 ---
    let (os8_coll, _os8_db, _os8_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "pq_os8",
        "TRAIN QUANTIZER ON pq_os8 WITH (m=8, k=256, type='pq', oversampling=8)",
    );
    let os8_recall = measure_recall(&os8_coll, &queries, &dataset, K);
    println!("PQ oversampling=8 recall@{K}: {os8_recall:.4}");

    // Benchmark group
    let mut group = c.benchmark_group("pq_recall");
    group.sample_size(10); // accuracy benchmark, not speed
    group.measurement_time(std::time::Duration::from_secs(5));

    group.bench_function("pq_recall_m8_k256_rescore", |b| {
        b.iter(|| {
            let recall = measure_recall(
                black_box(&pq_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
            );
            // PQ-07 contract: recall@10 >= 0.92 for m=8 k=256 with rescore
            assert!(
                recall >= 0.92,
                "PQ m=8 k=256 rescore recall@{K} = {recall:.4}, expected >= 0.92"
            );
            recall
        });
    });

    group.bench_function("full_precision_baseline", |b| {
        b.iter(|| {
            let recall = measure_recall(
                black_box(&full_collection),
                black_box(&queries),
                black_box(&dataset),
                K,
            );
            // Full precision HNSW baseline must exceed PQ threshold
            assert!(
                recall >= 0.95,
                "Full precision recall@{K} = {recall:.4}, expected >= 0.95"
            );
            recall
        });
    });

    group.bench_function("pq_recall_m8_k256_no_rescore", |b| {
        b.iter(|| {
            let recall = measure_recall(
                black_box(&norescore_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
            );
            assert!(
                recall >= 0.20,
                "PQ no-rescore recall@{K} = {recall:.4}, expected >= 0.20"
            );
            recall
        });
    });

    group.bench_function("opq_recall_m8_k256_rescore", |b| {
        b.iter(|| {
            let recall = measure_recall(
                black_box(&opq_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
            );
            // OPQ with rescore should match or exceed standard PQ recall
            assert!(
                recall >= 0.92,
                "OPQ m=8 k=256 rescore recall@{K} = {recall:.4}, expected >= 0.92"
            );
            recall
        });
    });

    group.bench_function("rabitq_recall_128d", |b| {
        b.iter(|| {
            let recall = measure_recall(
                black_box(&rabitq_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
            );
            assert!(
                recall >= 0.80,
                "RaBitQ 128d recall@{K} = {recall:.4}, expected >= 0.80"
            );
            recall
        });
    });

    group.bench_function("pq_recall_m8_k256_oversampling8", |b| {
        b.iter(|| {
            let recall = measure_recall(
                black_box(&os8_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
            );
            // 8x oversampling should match or exceed 4x recall
            assert!(
                recall >= 0.92,
                "PQ oversampling=8 recall@{K} = {recall:.4}, expected >= 0.92"
            );
            recall
        });
    });

    group.finish();

    // Print summary for CI reporting
    println!("\n=== PQ Recall Summary ===");
    println!("PQ m=8 k=256 rescore:  {pq_recall:.4}");
    println!("Full precision:        {full_recall:.4}");
    println!("PQ no-rescore:         {norescore_recall:.4}");
    println!("OPQ m=8 k=256 rescore: {opq_recall:.4}");
    println!("RaBitQ 128d:           {rabitq_recall:.4}");
    println!("PQ oversampling=8:     {os8_recall:.4}");
}

criterion_group!(pq_recall, pq_recall_benchmarks);
criterion_main!(pq_recall);
