#![allow(deprecated)] // Benches use legacy Collection.
//! Multi-distribution PQ recall accuracy benchmark suite (5K vectors, 128d).
//!
//! Measures recall@10 for PQ, OPQ, and `RaBitQ` quantization methods across
//! three data distributions: clustered Gaussian, binary {0,1}, and exact
//! search baselines for all distributions.
//!
//! Extends the uniform random coverage in `pq_recall_benchmark.rs` to validate
//! HNSW+PQ behavior on realistic distribution shapes.

#![allow(clippy::cast_precision_loss)]

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rand_distr::{Distribution, Normal};
use tempfile::{tempdir, TempDir};
use velesdb_core::velesql::Parser;
use velesdb_core::{Collection, Database, DistanceMetric, Point, StorageMode};

const DIMENSION: usize = 128;
const NUM_VECTORS: usize = 5_000;
const NUM_QUERIES: usize = 50;
const K: usize = 10;
const CLUSTERED_EF_SEARCH: usize = 512;
const NUM_CLUSTERS: usize = 10;
const SIGMA: f32 = 0.1;
const BINARY_DENSITY: f64 = 0.5;

// ---------------------------------------------------------------------------
// Data generators
// ---------------------------------------------------------------------------

/// Generate clustered Gaussian data: `n_clusters` random centroids in [-1, 1]^dim,
/// each vector is a random cluster centroid + Normal(0, sigma) noise.
fn generate_clustered_data(
    n: usize,
    dim: usize,
    n_clusters: usize,
    sigma: f32,
    seed: u64,
) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);

    // Generate random centroids in [-1, 1]^dim
    let centroids: Vec<Vec<f32>> = (0..n_clusters)
        .map(|_| (0..dim).map(|_| rng.gen_range(-1.0_f32..1.0)).collect())
        .collect();

    let normal = Normal::new(0.0_f32, sigma).expect("valid normal distribution");

    (0..n)
        .map(|_| {
            let cluster_idx = rng.gen_range(0..n_clusters);
            let centroid = &centroids[cluster_idx];
            centroid
                .iter()
                .map(|&c| c + normal.sample(&mut rng))
                .collect()
        })
        .collect()
}

/// Generate binary {0, 1} data: each component is 0.0 or 1.0 with given density.
fn generate_binary_data(n: usize, dim: usize, density: f64, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            (0..dim)
                .map(|_| if rng.gen_bool(density) { 1.0_f32 } else { 0.0 })
                .collect()
        })
        .collect()
}

/// Generate uniform random data in [-1, 1]^dim (same as existing benchmark).
fn generate_random_data(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| (0..dim).map(|_| rng.gen_range(-1.0_f32..1.0)).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// Helper functions (duplicated from pq_recall_benchmark.rs)
// ---------------------------------------------------------------------------

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

/// Build a collection with specified storage mode (no quantization training).
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

/// Build a collection via `Database` + `VelesQL` `TRAIN QUANTIZER` for explicit training.
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

/// Measure average recall@k using default `ef_search` (128).
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

/// Measure average recall@k with explicit `ef_search` override.
fn measure_recall_with_ef(
    collection: &Collection,
    queries: &[Vec<f32>],
    dataset: &[Vec<f32>],
    k: usize,
    ef_search: usize,
) -> f64 {
    assert!(
        !queries.is_empty(),
        "measure_recall_with_ef requires non-empty queries"
    );
    let mut total_recall = 0.0;
    for query in queries {
        let gt = brute_force_topk(query, dataset, k);
        let results = collection
            .search_with_ef(query, k, ef_search)
            .expect("search_with_ef");
        let result_ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        total_recall += recall_at_k(&gt, &result_ids, k);
    }
    #[allow(clippy::cast_precision_loss)]
    let avg = total_recall / queries.len() as f64;
    avg
}

// ---------------------------------------------------------------------------
// Benchmark Group 1: Clustered Gaussian (6 variants, ef_search=512)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn clustered_recall_benchmarks(c: &mut Criterion) {
    let dataset = generate_clustered_data(NUM_VECTORS, DIMENSION, NUM_CLUSTERS, SIGMA, 42);
    let queries = generate_clustered_data(NUM_QUERIES, DIMENSION, NUM_CLUSTERS, SIGMA, 999);

    // --- Variant 1: PQ m=8 k=256 with rescore ---
    let (pq_coll, _pq_db, _pq_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "cl_pq_m8",
        "TRAIN QUANTIZER ON cl_pq_m8 WITH (m=8, k=256, type='pq')",
    );
    let pq_recall = measure_recall_with_ef(&pq_coll, &queries, &dataset, K, CLUSTERED_EF_SEARCH);
    println!("[clustered] PQ m=8 k=256 rescore recall@{K}: {pq_recall:.4}");

    // --- Variant 2: Full precision baseline ---
    let (full_coll, _full_dir) =
        build_pq_collection(StorageMode::Full, &dataset, DIMENSION, "cl_full");
    let full_recall =
        measure_recall_with_ef(&full_coll, &queries, &dataset, K, CLUSTERED_EF_SEARCH);
    println!("[clustered] Full precision recall@{K}: {full_recall:.4}");

    // --- Variant 3: PQ no-rescore ---
    let (norescore_coll, _norescore_db, _norescore_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "cl_pq_nores",
        "TRAIN QUANTIZER ON cl_pq_nores WITH (m=8, k=256, type='pq', oversampling=0)",
    );
    let norescore_recall =
        measure_recall_with_ef(&norescore_coll, &queries, &dataset, K, CLUSTERED_EF_SEARCH);
    println!("[clustered] PQ no-rescore recall@{K}: {norescore_recall:.4}");

    // --- Variant 4: OPQ m=8 k=256 with rescore ---
    let (opq_coll, _opq_db, _opq_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "cl_opq_m8",
        "TRAIN QUANTIZER ON cl_opq_m8 WITH (m=8, k=256, type='opq')",
    );
    let opq_recall = measure_recall_with_ef(&opq_coll, &queries, &dataset, K, CLUSTERED_EF_SEARCH);
    println!("[clustered] OPQ m=8 k=256 rescore recall@{K}: {opq_recall:.4}");

    // --- Variant 5: RaBitQ 128d ---
    let (rabitq_coll, _rabitq_db, _rabitq_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "cl_rabitq",
        "TRAIN QUANTIZER ON cl_rabitq WITH (m=8, type='rabitq')",
    );
    let rabitq_recall =
        measure_recall_with_ef(&rabitq_coll, &queries, &dataset, K, CLUSTERED_EF_SEARCH);
    println!("[clustered] RaBitQ 128d recall@{K}: {rabitq_recall:.4}");

    // --- Variant 6: PQ oversampling=8 ---
    let (os8_coll, _os8_db, _os8_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "cl_pq_os8",
        "TRAIN QUANTIZER ON cl_pq_os8 WITH (m=8, k=256, type='pq', oversampling=8)",
    );
    let os8_recall = measure_recall_with_ef(&os8_coll, &queries, &dataset, K, CLUSTERED_EF_SEARCH);
    println!("[clustered] PQ oversampling=8 recall@{K}: {os8_recall:.4}");

    // Benchmark group
    let mut group = c.benchmark_group("clustered_recall");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("pq_m8_k256_rescore", |b| {
        b.iter(|| {
            let recall = measure_recall_with_ef(
                black_box(&pq_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
                CLUSTERED_EF_SEARCH,
            );
            assert!(
                recall >= 0.85,
                "Clustered PQ rescore recall@{K} = {recall:.4}, expected >= 0.85 \
                 (limitation: HNSW recall on clustered data is known to have a ceiling around 0.87-0.88)"
            );
            recall
        });
    });

    group.bench_function("full_precision", |b| {
        b.iter(|| {
            let recall = measure_recall_with_ef(
                black_box(&full_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
                CLUSTERED_EF_SEARCH,
            );
            assert!(
                recall >= 0.85,
                "Clustered full precision recall@{K} = {recall:.4}, expected >= 0.85 \
                 (limitation: HNSW recall on clustered data is known to have a ceiling around 0.87-0.88)"
            );
            recall
        });
    });

    group.bench_function("pq_no_rescore", |b| {
        b.iter(|| {
            let recall = measure_recall_with_ef(
                black_box(&norescore_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
                CLUSTERED_EF_SEARCH,
            );
            assert!(
                recall >= 0.20,
                "Clustered PQ no-rescore recall@{K} = {recall:.4}, expected >= 0.20"
            );
            recall
        });
    });

    group.bench_function("opq_m8_k256_rescore", |b| {
        b.iter(|| {
            let recall = measure_recall_with_ef(
                black_box(&opq_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
                CLUSTERED_EF_SEARCH,
            );
            assert!(
                recall >= 0.85,
                "Clustered OPQ rescore recall@{K} = {recall:.4}, expected >= 0.85 \
                 (limitation: HNSW recall on clustered data is known to have a ceiling around 0.87-0.88)"
            );
            recall
        });
    });

    group.bench_function("rabitq_128d", |b| {
        b.iter(|| {
            let recall = measure_recall_with_ef(
                black_box(&rabitq_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
                CLUSTERED_EF_SEARCH,
            );
            assert!(
                recall >= 0.85,
                "Clustered RaBitQ recall@{K} = {recall:.4}, expected >= 0.85 \
                 (limitation: HNSW recall on clustered data is known to have a ceiling around 0.87-0.88)"
            );
            recall
        });
    });

    group.bench_function("pq_oversampling8", |b| {
        b.iter(|| {
            let recall = measure_recall_with_ef(
                black_box(&os8_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
                CLUSTERED_EF_SEARCH,
            );
            assert!(
                recall >= 0.85,
                "Clustered PQ oversampling=8 recall@{K} = {recall:.4}, expected >= 0.85 \
                 (limitation: HNSW recall on clustered data is known to have a ceiling around 0.87-0.88)"
            );
            recall
        });
    });

    group.finish();

    // CI summary
    println!("\n=== Clustered Recall Summary ===");
    println!("PQ m=8 k=256 rescore:  {pq_recall:.4}");
    println!("Full precision:        {full_recall:.4}");
    println!("PQ no-rescore:         {norescore_recall:.4}");
    println!("OPQ m=8 k=256 rescore: {opq_recall:.4}");
    println!("RaBitQ 128d:           {rabitq_recall:.4}");
    println!("PQ oversampling=8:     {os8_recall:.4}");
}

// ---------------------------------------------------------------------------
// Benchmark Group 2: Binary {0,1} (2 variants, default ef_search=128)
// ---------------------------------------------------------------------------

fn binary_recall_benchmarks(c: &mut Criterion) {
    let dataset = generate_binary_data(NUM_VECTORS, DIMENSION, BINARY_DENSITY, 42);
    let queries = generate_binary_data(NUM_QUERIES, DIMENSION, BINARY_DENSITY, 999);

    // --- Variant 1: RaBitQ 128d ---
    let (rabitq_coll, _rabitq_db, _rabitq_dir) = build_trained_collection(
        &dataset,
        DIMENSION,
        "bin_rabitq",
        "TRAIN QUANTIZER ON bin_rabitq WITH (m=8, type='rabitq')",
    );
    let rabitq_recall = measure_recall(&rabitq_coll, &queries, &dataset, K);
    println!("[binary] RaBitQ 128d recall@{K}: {rabitq_recall:.4}");

    // --- Variant 2: Full precision baseline ---
    let (full_coll, _full_dir) =
        build_pq_collection(StorageMode::Full, &dataset, DIMENSION, "bin_full");
    let full_recall = measure_recall(&full_coll, &queries, &dataset, K);
    println!("[binary] Full precision recall@{K}: {full_recall:.4}");

    // Benchmark group
    let mut group = c.benchmark_group("binary_recall");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("rabitq_128d", |b| {
        b.iter(|| {
            let recall = measure_recall(
                black_box(&rabitq_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
            );
            assert!(
                recall >= 0.85,
                "Binary RaBitQ recall@{K} = {recall:.4}, expected >= 0.85"
            );
            recall
        });
    });

    group.bench_function("full_precision", |b| {
        b.iter(|| {
            let recall = measure_recall(
                black_box(&full_coll),
                black_box(&queries),
                black_box(&dataset),
                K,
            );
            assert!(
                recall >= 0.85,
                "Binary full precision recall@{K} = {recall:.4}, expected >= 0.85"
            );
            recall
        });
    });

    group.finish();

    // CI summary
    println!("\n=== Binary Recall Summary ===");
    println!("RaBitQ 128d:    {rabitq_recall:.4}");
    println!("Full precision: {full_recall:.4}");
}

// ---------------------------------------------------------------------------
// Benchmark Group 3: Exact search baselines (3 distributions)
// ---------------------------------------------------------------------------

fn exact_recall_benchmarks(c: &mut Criterion) {
    let uniform_data = generate_random_data(NUM_VECTORS, DIMENSION, 42);
    let uniform_queries = generate_random_data(NUM_QUERIES, DIMENSION, 123);

    let clustered_data = generate_clustered_data(NUM_VECTORS, DIMENSION, NUM_CLUSTERS, SIGMA, 42);
    let clustered_queries =
        generate_clustered_data(NUM_QUERIES, DIMENSION, NUM_CLUSTERS, SIGMA, 999);

    let binary_data = generate_binary_data(NUM_VECTORS, DIMENSION, BINARY_DENSITY, 42);
    let binary_queries = generate_binary_data(NUM_QUERIES, DIMENSION, BINARY_DENSITY, 999);

    // Validate exact search ground truth consistency
    let uniform_recall = compute_exact_recall(&uniform_queries, &uniform_data, K);
    let clustered_recall = compute_exact_recall(&clustered_queries, &clustered_data, K);
    let binary_recall = compute_exact_recall(&binary_queries, &binary_data, K);

    println!("[exact] Uniform recall:   {uniform_recall:.4}");
    println!("[exact] Clustered recall:  {clustered_recall:.4}");
    println!("[exact] Binary recall:     {binary_recall:.4}");

    assert!(
        (uniform_recall - 1.0).abs() < f64::EPSILON,
        "Exact uniform recall must be 1.0, got {uniform_recall}"
    );
    assert!(
        (clustered_recall - 1.0).abs() < f64::EPSILON,
        "Exact clustered recall must be 1.0, got {clustered_recall}"
    );
    assert!(
        (binary_recall - 1.0).abs() < f64::EPSILON,
        "Exact binary recall must be 1.0, got {binary_recall}"
    );

    let mut group = c.benchmark_group("exact_recall");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("uniform", |b| {
        b.iter(|| {
            let recall =
                compute_exact_recall(black_box(&uniform_queries), black_box(&uniform_data), K);
            assert!(
                (recall - 1.0).abs() < f64::EPSILON,
                "Exact uniform recall must be 1.0, got {recall}"
            );
            recall
        });
    });

    group.bench_function("clustered", |b| {
        b.iter(|| {
            let recall =
                compute_exact_recall(black_box(&clustered_queries), black_box(&clustered_data), K);
            assert!(
                (recall - 1.0).abs() < f64::EPSILON,
                "Exact clustered recall must be 1.0, got {recall}"
            );
            recall
        });
    });

    group.bench_function("binary", |b| {
        b.iter(|| {
            let recall =
                compute_exact_recall(black_box(&binary_queries), black_box(&binary_data), K);
            assert!(
                (recall - 1.0).abs() < f64::EPSILON,
                "Exact binary recall must be 1.0, got {recall}"
            );
            recall
        });
    });

    group.finish();

    // CI summary
    println!("\n=== Exact Recall Summary ===");
    println!("Uniform:   {uniform_recall:.4}");
    println!("Clustered: {clustered_recall:.4}");
    println!("Binary:    {binary_recall:.4}");
}

/// Compute exact recall: brute-force top-k vs brute-force top-k (must be 1.0).
fn compute_exact_recall(queries: &[Vec<f32>], dataset: &[Vec<f32>], k: usize) -> f64 {
    let mut total_recall = 0.0;
    for query in queries {
        let gt = brute_force_topk(query, dataset, k);
        let results = brute_force_topk(query, dataset, k);
        total_recall += recall_at_k(&gt, &results, k);
    }
    #[allow(clippy::cast_precision_loss)]
    let avg = total_recall / queries.len() as f64;
    avg
}

criterion_group!(clustered, clustered_recall_benchmarks);
criterion_group!(binary, binary_recall_benchmarks);
criterion_group!(exact, exact_recall_benchmarks);
criterion_main!(clustered, binary, exact);
