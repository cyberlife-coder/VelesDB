//! Benchmark suite for VelesDB-Core search operations.
//!
//! Run with: `cargo bench --all-features`

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn generate_random_vector(dim: usize) -> Vec<f32> {
    (0..dim).map(|i| (i as f32 * 0.1).sin()).collect()
}

fn bench_vector_distance(c: &mut Criterion) {
    let dim = 768;
    let vec_a = generate_random_vector(dim);
    let vec_b = generate_random_vector(dim);

    c.bench_function("cosine_distance_768d", |b| {
        b.iter(|| {
            let dot: f32 = vec_a.iter().zip(&vec_b).map(|(a, b)| a * b).sum();
            let norm_a: f32 = vec_a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm_b: f32 = vec_b.iter().map(|x| x * x).sum::<f32>().sqrt();
            black_box(1.0 - dot / (norm_a * norm_b))
        });
    });

    c.bench_function("euclidean_distance_768d", |b| {
        b.iter(|| {
            let sum: f32 = vec_a
                .iter()
                .zip(&vec_b)
                .map(|(a, b)| (a - b).powi(2))
                .sum();
            black_box(sum.sqrt())
        });
    });
}

fn bench_vector_normalization(c: &mut Criterion) {
    let dim = 768;
    let vec = generate_random_vector(dim);

    c.bench_function("normalize_768d", |b| {
        b.iter(|| {
            let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
            let normalized: Vec<f32> = vec.iter().map(|x| x / norm).collect();
            black_box(normalized)
        });
    });
}

criterion_group!(benches, bench_vector_distance, bench_vector_normalization);
criterion_main!(benches);
