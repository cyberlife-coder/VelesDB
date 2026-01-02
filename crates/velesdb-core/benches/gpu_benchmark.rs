//! GPU vs CPU SIMD benchmark for batch cosine similarity.
//!
//! Run with: `cargo bench --bench gpu_benchmark --features gpu`
//!
//! Compares GPU (wgpu) vs CPU SIMD for batch operations.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::simd;

#[cfg(feature = "gpu")]
use velesdb_core::gpu::GpuAccelerator;

fn generate_random_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    (0..dim)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            #[allow(clippy::cast_precision_loss)]
            let val = ((state >> 16) & 0x7FFF) as f32 / 32768.0;
            val * 2.0 - 1.0
        })
        .collect()
}

/// Benchmark batch cosine similarity: CPU SIMD vs GPU
fn bench_batch_cosine(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_cosine_similarity");

    let dim = 768;
    let batch_sizes = [100, 1000, 10000];

    for &batch_size in &batch_sizes {
        // Generate data
        let query = generate_random_vector(dim, 0);
        let vectors: Vec<f32> = (0..batch_size)
            .flat_map(|i| generate_random_vector(dim, i as u64 + 1))
            .collect();

        // CPU SIMD baseline
        group.bench_function(
            BenchmarkId::new("cpu_simd", format!("{batch_size}x{dim}")),
            |b| {
                b.iter(|| {
                    let results: Vec<f32> = (0..batch_size)
                        .map(|i| {
                            let start = i * dim;
                            let end = start + dim;
                            simd::cosine_similarity_fast(&vectors[start..end], &query)
                        })
                        .collect();
                    black_box(results)
                });
            },
        );

        // GPU benchmark (if available)
        #[cfg(feature = "gpu")]
        {
            if let Some(gpu) = GpuAccelerator::new() {
                group.bench_function(
                    BenchmarkId::new("gpu_wgpu", format!("{batch_size}x{dim}")),
                    |b| {
                        b.iter(|| {
                            let results = gpu.batch_cosine_similarity(&vectors, &query, dim);
                            black_box(results)
                        });
                    },
                );
            } else {
                println!("GPU not available, skipping GPU benchmark");
            }
        }
    }

    group.finish();
}

/// Benchmark single vector cosine: CPU should always win
fn bench_single_cosine(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_cosine_similarity");

    let dim = 768;
    let query = generate_random_vector(dim, 0);
    let vector = generate_random_vector(dim, 1);

    // CPU SIMD - should be ~83ns
    group.bench_function(BenchmarkId::new("cpu_simd", format!("{dim}d")), |b| {
        b.iter(|| black_box(simd::cosine_similarity_fast(&vector, &query)));
    });

    // GPU for single vector (to show overhead)
    #[cfg(feature = "gpu")]
    {
        if let Some(gpu) = GpuAccelerator::new() {
            group.bench_function(BenchmarkId::new("gpu_wgpu", format!("{dim}d")), |b| {
                b.iter(|| {
                    let results = gpu.batch_cosine_similarity(&vector, &query, dim);
                    black_box(results)
                });
            });
        }
    }

    group.finish();
}

criterion_group!(benches, bench_batch_cosine, bench_single_cosine);
criterion_main!(benches);
