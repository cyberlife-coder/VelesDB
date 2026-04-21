//! GPU traversal benchmark: CPU vs GPU HNSW layer-0 search at scale.
//!
//! Run with: `cargo bench --bench gpu_traversal_benchmark --features gpu`
//!
//! Benchmarks the SONG 3-stage GPU pipeline against CPU BFS traversal
//! at 1M, 5M, and 10M vector scales (Issue #502 requirement).
//!
//! **Note**: 5M and 10M benchmarks require significant RAM (~30GB for 10M×768).
//! They are gated behind `internal-bench` feature to avoid accidental CI runs.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::time::Duration;

#[cfg(feature = "gpu")]
use velesdb_core::gpu::gpu_csr::CsrGraph;
#[cfg(feature = "gpu")]
use velesdb_core::gpu::gpu_traversal::{should_traverse_gpu, GpuTraversalContext};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deterministic pseudo-random vector generator (no dependency on `rand`).
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    (0..dim)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            #[allow(clippy::cast_precision_loss)]
            let val = ((state >> 33) & 0xFFFF) as f32 / 65536.0;
            val * 2.0 - 1.0
        })
        .collect()
}

/// Builds a synthetic HNSW-like graph in CSR format.
///
/// Each node connects to `degree` random neighbors using a deterministic
/// hash function. This approximates the connectivity of a real HNSW layer-0
/// graph without requiring actual index construction.
#[cfg(feature = "gpu")]
fn build_synthetic_csr(num_nodes: usize, degree: usize) -> CsrGraph {
    let mut offsets = Vec::with_capacity(num_nodes + 1);
    let mut neighbors = Vec::with_capacity(num_nodes * degree);

    offsets.push(0u32);
    for node in 0..num_nodes {
        for d in 0..degree {
            // Deterministic neighbor: hash(node, d) % num_nodes
            let seed = (node as u64)
                .wrapping_mul(2_654_435_761)
                .wrapping_add(d as u64);
            #[allow(clippy::cast_possible_truncation)]
            let neighbor = (seed % num_nodes as u64) as u32;
            neighbors.push(neighbor);
        }
        #[allow(clippy::cast_possible_truncation)]
        let offset = neighbors.len() as u32;
        offsets.push(offset);
    }

    CsrGraph {
        offsets,
        neighbors,
        num_nodes: num_nodes as u32,
        max_degree: degree as u32,
        total_edges: (num_nodes * degree) as u32,
    }
}

/// Builds flat f32 vector storage for `num_vectors` vectors of `dim` dimensions.
fn build_flat_vectors(num_vectors: usize, dim: usize) -> Vec<f32> {
    let mut vectors = Vec::with_capacity(num_vectors * dim);
    for i in 0..num_vectors {
        vectors.extend(generate_vector(dim, i as u64));
    }
    vectors
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// Core benchmark: GPU traversal at different scales.
///
/// Tests the full SONG pipeline (Expand → Distance → Select) × iterations.
#[cfg(feature = "gpu")]
fn bench_gpu_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("gpu_hnsw_traversal");
    group.sample_size(10); // Large datasets, fewer samples
    group.measurement_time(Duration::from_secs(30));

    let dim = 768; // Standard embedding dimension
    let degree = 32; // Typical HNSW M0 = 2*M = 32
    let k = 10;
    let ef_search = 128;

    // 1M benchmark (always run)
    let scales: Vec<(usize, &str)> = {
        let mut s = vec![(1_000_000, "1M")];

        // 5M and 10M only with internal-bench feature (requires ~30GB RAM)
        #[cfg(feature = "internal-bench")]
        {
            s.push((5_000_000, "5M"));
            s.push((10_000_000, "10M"));
        }

        s
    };

    // Check GPU availability
    let ctx = match GpuTraversalContext::new() {
        Some(ctx) => ctx,
        None => {
            eprintln!("⚠ GPU not available — skipping GPU traversal benchmarks");
            group.finish();
            return;
        }
    };

    for (num_vectors, label) in &scales {
        // Verify GPU should activate at this scale (bench dims always fit u32).
        assert!(
            should_traverse_gpu(*num_vectors, dim),
            "GPU should activate at {label} vectors"
        );

        eprintln!(
            "Building {label} synthetic graph ({num_vectors} nodes, degree={degree}, dim={dim})..."
        );
        let csr = build_synthetic_csr(*num_vectors, degree);
        let vectors = build_flat_vectors(*num_vectors, dim);
        let query = generate_vector(dim, 42);

        // Validate CSR before GPU upload
        csr.validate()
            .unwrap_or_else(|e| panic!("CSR validation failed for {label}: {e}"));

        let vram_mb = (csr.total_gpu_bytes() + vectors.len() * 4) / (1024 * 1024);
        eprintln!(
            "  CSR: {} edges, max_deg={}, VRAM estimate: {}MB",
            csr.total_edges, csr.max_degree, vram_mb
        );

        // GPU traversal benchmark
        group.bench_function(
            BenchmarkId::new("gpu_song", format!("{label}_ef{ef_search}")),
            |b| {
                b.iter(|| {
                    let results = ctx.search_layer0(
                        &csr,
                        &vectors,
                        &query,
                        0, // entry_node
                        k,
                        ef_search,
                        dim,
                        velesdb_core::distance::DistanceMetric::Cosine,
                    );
                    black_box(results)
                });
            },
        );

        // Also benchmark with different ef values
        for ef in [64, 256] {
            group.bench_function(
                BenchmarkId::new("gpu_song", format!("{label}_ef{ef}")),
                |b| {
                    b.iter(|| {
                        let results = ctx.search_layer0(
                            &csr,
                            &vectors,
                            &query,
                            0,
                            k,
                            ef,
                            dim,
                            velesdb_core::distance::DistanceMetric::Cosine,
                        );
                        black_box(results)
                    });
                },
            );
        }
    }

    group.finish();
}

/// Benchmark the CSR construction cost (CPU-only, no GPU needed).
///
/// This measures the overhead of converting HNSW Layer adjacency lists
/// to the flat CSR format for GPU upload.
fn bench_csr_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("csr_construction");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));

    let degree = 32;

    for (num_nodes, label) in [(100_000, "100K"), (500_000, "500K"), (1_000_000, "1M")] {
        // We can't use Layer directly from benches (it's pub(crate)),
        // so we benchmark the synthetic CSR builder which has the same
        // O(N × degree) complexity as CsrGraph::from_layer().
        #[cfg(feature = "gpu")]
        group.bench_function(
            BenchmarkId::new("build_csr", format!("{label}_deg{degree}")),
            |b| {
                b.iter(|| {
                    let csr = build_synthetic_csr(num_nodes, degree);
                    black_box(csr)
                });
            },
        );

        let _ = (num_nodes, label); // Silence unused warning when gpu feature is off
    }

    group.finish();
}

/// Benchmark GPU activation threshold decision.
#[cfg(feature = "gpu")]
fn bench_threshold_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("gpu_threshold");

    for size in [100_000, 500_000, 500_001, 1_000_000, 10_000_000] {
        group.bench_function(BenchmarkId::new("should_traverse_gpu", size), |b| {
            b.iter(|| black_box(should_traverse_gpu(black_box(size), black_box(128))));
        });
    }

    group.finish();
}

#[cfg(feature = "gpu")]
criterion_group!(
    benches,
    bench_gpu_traversal,
    bench_csr_construction,
    bench_threshold_check,
);

#[cfg(not(feature = "gpu"))]
criterion_group!(benches, bench_csr_construction);

criterion_main!(benches);
