//! Benchmark suite for `NEAR_FUSED` multi-vector fused search (VP-012, Plan 06-04).
//!
//! Run with: `cargo bench --bench near_fused_bench`
//!
//! Measures:
//! - Single vector search baseline
//! - `NEAR_FUSED` with 2 vectors (RRF, Average)
//! - `NEAR_FUSED` with 3 vectors (RRF)
//! - Fusion overhead percentage

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::json;
use tempfile::tempdir;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::fusion::FusionStrategy;
use velesdb_core::{Collection, Database, Point};

/// Dimension of test vectors.
const DIM: usize = 128;

/// Create a collection with `n` random-ish points of dimension `DIM`.
fn setup_collection(n: usize) -> (tempfile::TempDir, Collection) {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    db.create_collection("bench", DIM, DistanceMetric::Cosine)
        .expect("create");
    let col = db.get_collection("bench").expect("get");

    let points: Vec<Point> = (0..n)
        .map(|i| {
            // Deterministic pseudo-random vector
            #[allow(clippy::cast_precision_loss)]
            let vec: Vec<f32> = (0..DIM)
                .map(|d| ((i * 7 + d * 13) % 1000) as f32 / 1000.0)
                .collect();
            Point::new(u64::try_from(i).expect("id"), vec, Some(json!({"idx": i})))
        })
        .collect();

    col.upsert(points).expect("upsert");
    (dir, col)
}

#[allow(clippy::cast_precision_loss)]
fn query_vec(seed: usize) -> Vec<f32> {
    let v: Vec<f32> = (0..DIM)
        .map(|d| ((seed * 11 + d * 17) % 1000) as f32 / 1000.0)
        .collect();
    v
}

fn bench_single_vector_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("near_fused_baseline");

    for &n in &[1_000, 10_000] {
        let (_dir, col) = setup_collection(n);
        let q = query_vec(42);

        group.bench_function(BenchmarkId::new("single_near", n), |b| {
            b.iter(|| black_box(col.search(black_box(&q), 10).unwrap()));
        });
    }

    group.finish();
}

fn bench_near_fused_rrf(c: &mut Criterion) {
    let mut group = c.benchmark_group("near_fused_rrf");

    for &n in &[1_000, 10_000] {
        let (_dir, col) = setup_collection(n);
        let q1 = query_vec(42);
        let q2 = query_vec(99);
        let q3 = query_vec(7);

        group.bench_function(BenchmarkId::new("2vec_rrf", n), |b| {
            b.iter(|| {
                let vecs: Vec<&[f32]> = vec![&q1, &q2];
                black_box(
                    col.multi_query_search(&vecs, 10, FusionStrategy::RRF { k: 60 }, None)
                        .unwrap(),
                )
            });
        });

        group.bench_function(BenchmarkId::new("3vec_rrf", n), |b| {
            b.iter(|| {
                let vecs: Vec<&[f32]> = vec![&q1, &q2, &q3];
                black_box(
                    col.multi_query_search(&vecs, 10, FusionStrategy::RRF { k: 60 }, None)
                        .unwrap(),
                )
            });
        });
    }

    group.finish();
}

fn bench_near_fused_average(c: &mut Criterion) {
    let mut group = c.benchmark_group("near_fused_average");

    for &n in &[1_000, 10_000] {
        let (_dir, col) = setup_collection(n);
        let q1 = query_vec(42);
        let q2 = query_vec(99);

        group.bench_function(BenchmarkId::new("2vec_avg", n), |b| {
            b.iter(|| {
                let vecs: Vec<&[f32]> = vec![&q1, &q2];
                black_box(
                    col.multi_query_search(&vecs, 10, FusionStrategy::Average, None)
                        .unwrap(),
                )
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_single_vector_search,
    bench_near_fused_rrf,
    bench_near_fused_average,
);
criterion_main!(benches);
