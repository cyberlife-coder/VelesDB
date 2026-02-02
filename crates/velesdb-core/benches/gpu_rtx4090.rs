//! Benchmark GPU RTX 4090 vs CPU i9-14900K
//! Comparaison des performances CPU AVX-512 vs GPU CUDA

#![allow(clippy::cast_precision_loss)] // Reason: Benchmark test data generation, precision loss acceptable

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

#[cfg(feature = "gpu")]
fn gpu_vs_cpu_rtx4090(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtx4090_gpu_vs_cpu");

    for size in [384usize, 768, 1536, 3072, 4096] {
        let a: Vec<f32> = (0..size).map(|i| (i as f32) * 0.001).collect();
        let b: Vec<f32> = (0..size).map(|i| ((size - 1 - i) as f32) * 0.001).collect();

        // CPU AVX-512 (i9-14900K)
        group.bench_with_input(
            BenchmarkId::new("cpu_avx512", size),
            &(a.clone(), b.clone()),
            |bench, (a, b)| {
                bench.iter(|| black_box(velesdb_core::simd_native::dot_product_native(a, b)));
            },
        );

        // GPU wgpu (RTX 4090)
        #[cfg(feature = "gpu")]
        group.bench_with_input(
            BenchmarkId::new("gpu_wgpu", size),
            &(a.clone(), b.clone()),
            |bench, (a, b)| {
                bench.iter(|| {
                    // TODO: Implement GPU dot product benchmark
                    black_box(velesdb_core::simd_native::dot_product_native(a, b))
                });
            },
        );
    }

    group.finish();
}

#[cfg(not(feature = "gpu"))]
fn cpu_only_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("rtx4090_cpu_only");

    println!(
        "ℹ️  GPU feature not enabled. Run with: cargo bench --bench gpu_rtx4090 --features gpu"
    );

    for size in [384usize, 768, 1536, 3072, 4096] {
        let a: Vec<f32> = (0..size).map(|i| (i as f32) * 0.001).collect();
        let b: Vec<f32> = (0..size).map(|i| ((size - 1 - i) as f32) * 0.001).collect();

        group.bench_with_input(
            BenchmarkId::new("cpu_avx512", size),
            &(a, b),
            |bench, (a, b)| {
                bench.iter(|| black_box(velesdb_core::simd_native::dot_product_native(a, b)));
            },
        );
    }

    group.finish();
}

#[cfg(feature = "gpu")]
criterion_group!(benches, gpu_vs_cpu_rtx4090);
#[cfg(not(feature = "gpu"))]
criterion_group!(benches, cpu_only_benchmark);
criterion_main!(benches);
