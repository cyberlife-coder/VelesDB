//! Diagnostic: Vérifier la détection SIMD réelle sur cette machine
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::uninlined_format_args)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn diagnose_simd(c: &mut Criterion) {
    // 1. Quel SIMD est détecté?
    let level = velesdb_core::simd_native::simd_level();
    println!("\n=== SIMD Detection Diagnostic ===");
    println!("Detected SIMD level: {:?}", level);

    // 2. Vérifier AVX-512 via is_x86_feature_detected
    #[cfg(target_arch = "x86_64")]
    {
        println!("avx512f: {}", is_x86_feature_detected!("avx512f"));
        println!("avx2: {}", is_x86_feature_detected!("avx2"));
        println!("fma: {}", is_x86_feature_detected!("fma"));
        println!("avx512vl: {}", is_x86_feature_detected!("avx512vl"));
        println!("avx512dq: {}", is_x86_feature_detected!("avx512dq"));
    }

    // 3. Benchmark pour confirmer quel code path est pris
    let size = 1536usize;
    let a: Vec<f32> = (0..size).map(|i| (i as f32) * 0.001).collect();
    let b: Vec<f32> = (0..size).map(|i| ((size - 1 - i) as f32) * 0.001).collect();

    c.bench_function("actual_dot_product_1536d", |bencher| {
        bencher.iter(|| black_box(velesdb_core::simd_native::dot_product_native(&a, &b)));
    });

    println!("=== End Diagnostic ===\n");
}

criterion_group!(benches, diagnose_simd);
criterion_main!(benches);
