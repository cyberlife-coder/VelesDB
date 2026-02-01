//! Benchmark comparaison AVX-512 natif vs AVX2 sur i9-14900K
//!
//! Objectif: mesurer le gain réel de l'AVX-512 natif sur ce CPU

#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn avx512_vs_avx2_i9_14900k(c: &mut Criterion) {
    let mut group = c.benchmark_group("i9-14900k_avx512_vs_avx2");

    // Forcer l'utilisation d'une implémentation spécifique
    for size in [384usize, 768, 1536, 3072] {
        let a: Vec<f32> = (0..size).map(|i| (i as f32) * 0.001).collect();
        let b: Vec<f32> = (0..size).map(|i| ((size - 1 - i) as f32) * 0.001).collect();

        // Via dispatch natif (AVX-512 sur i9-14900K)
        group.bench_with_input(
            BenchmarkId::new("native_dispatch", size),
            &(a.clone(), b.clone()),
            |bench, (a, b)| {
                bench.iter(|| black_box(velesdb_core::simd_native::dot_product_native(a, b)));
            },
        );
    }

    group.finish();
}

fn simd_level_detection(c: &mut Criterion) {
    c.bench_function("detect_simd_level_cached", |b| {
        // Warm up the cache
        let _ = velesdb_core::simd_native::simd_level();

        b.iter(|| black_box(velesdb_core::simd_native::simd_level()));
    });
}

criterion_group!(benches, avx512_vs_avx2_i9_14900k, simd_level_detection);
criterion_main!(benches);
