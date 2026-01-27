//! Fast performance smoke test for CI.
//!
//! EPIC-026/US-002: Runs in < 2 minutes on typical CI runner.
//! Designed for quick regression detection, not comprehensive benchmarking.
//!
//! # Usage
//!
//! ```bash
//! # Run smoke test
//! cargo bench --bench smoke_test -- --noplot
//!
//! # Save baseline
//! cargo bench --bench smoke_test -- --save-baseline baseline --noplot
//!
//! # Compare with baseline
//! cargo bench --bench smoke_test -- --baseline baseline --noplot
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::{Rng, SeedableRng};
use tempfile::TempDir;
use velesdb_core::{Collection, DistanceMetric, Point};

const SMOKE_VECTORS: usize = 10_000;
const SMOKE_DIM: usize = 128;
const SMOKE_QUERIES: usize = 100;
const SMOKE_K: usize = 10;

fn generate_deterministic_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    (0..dim).map(|_| rng.gen::<f32>() * 2.0 - 1.0).collect()
}

fn generate_deterministic_vectors(count: usize, dim: usize, base_seed: u64) -> Vec<Vec<f32>> {
    (0..count)
        .map(|i| generate_deterministic_vector(dim, base_seed + i as u64))
        .collect()
}

fn smoke_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("smoke_insert");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function(BenchmarkId::new("10k", "128d"), |b| {
        let vectors = generate_deterministic_vectors(SMOKE_VECTORS, SMOKE_DIM, 42);

        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let collection =
                Collection::create(dir.path().to_path_buf(), SMOKE_DIM, DistanceMetric::Cosine)
                    .unwrap();

            let points: Vec<Point> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| Point::without_payload(i as u64, v.clone()))
                .collect();

            collection.upsert(points).unwrap();

            black_box(collection.len())
        });
    });

    group.finish();
}

fn smoke_search(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let collection =
        Collection::create(dir.path().to_path_buf(), SMOKE_DIM, DistanceMetric::Cosine).unwrap();

    let vectors = generate_deterministic_vectors(SMOKE_VECTORS, SMOKE_DIM, 42);
    let points: Vec<Point> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| Point::without_payload(i as u64, v.clone()))
        .collect();
    collection.upsert(points).unwrap();

    let queries = generate_deterministic_vectors(SMOKE_QUERIES, SMOKE_DIM, 12345);

    let mut group = c.benchmark_group("smoke_search");
    group.sample_size(50);
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function(BenchmarkId::new("10k_k10", "128d"), |b| {
        let mut query_idx = 0;
        b.iter(|| {
            let query = &queries[query_idx % queries.len()];
            query_idx += 1;
            let results = collection.search(query, SMOKE_K).unwrap();
            black_box(results)
        });
    });

    group.finish();
}

fn smoke_hybrid(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let collection =
        Collection::create(dir.path().to_path_buf(), SMOKE_DIM, DistanceMetric::Cosine).unwrap();

    let vectors = generate_deterministic_vectors(1000, SMOKE_DIM, 42);
    let points: Vec<Point> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            // i is bounded by SMOKE_COUNT (1000), safe to cast to u16/u64
            #[allow(clippy::cast_possible_truncation)]
            let score = f64::from(i as u16) / 1000.0;
            let payload = serde_json::json!({
                "category": if i % 2 == 0 { "tech" } else { "science" },
                "score": score,
            });
            #[allow(clippy::cast_possible_truncation)]
            let id = i as u64;
            Point::new(id, v.clone(), Some(payload))
        })
        .collect();
    collection.upsert(points).unwrap();

    let queries = generate_deterministic_vectors(10, SMOKE_DIM, 12345);

    let mut group = c.benchmark_group("smoke_hybrid");
    group.sample_size(50);

    group.bench_function("vector_plus_filter", |b| {
        let mut query_idx = 0;
        b.iter(|| {
            let query = &queries[query_idx % queries.len()];
            query_idx += 1;

            let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
                field: "category".to_string(),
                value: serde_json::json!("tech"),
            });

            let results = collection
                .search_with_filter(query, SMOKE_K, &filter)
                .unwrap();
            black_box(results)
        });
    });

    group.finish();
}

criterion_group!(
    name = smoke;
    config = Criterion::default()
        .without_plots()
        .warm_up_time(std::time::Duration::from_secs(1));
    targets = smoke_insert, smoke_search, smoke_hybrid
);
criterion_main!(smoke);
