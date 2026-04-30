//! Flamegraph profiling target for HNSW batch insert.
//!
//! Usage: `cargo flamegraph --example profile_batch_insert -p velesdb-core --features persistence`
//!
//! Generates 10K × 768D vectors and inserts via `insert_batch_parallel()`.
//! In-memory only (no persistence) to isolate CPU profile.

#![allow(clippy::cast_precision_loss)]

use std::time::Instant;
use velesdb_core::{DistanceMetric, HnswIndex};

const NUM_VECTORS: u64 = 10_000;
const DIMENSION: usize = 768;

fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| (seed as f32 * 0.1 + i as f32 * 0.01).sin().midpoint(1.0))
        .collect()
}

fn main() {
    let index = HnswIndex::new(DIMENSION, DistanceMetric::Cosine)
        .expect("invariant: valid dimension and metric");

    let vectors: Vec<(u64, Vec<f32>)> = (0..NUM_VECTORS)
        .map(|i| (i, generate_vector(DIMENSION, i)))
        .collect();

    let start = Instant::now();
    let inserted = index.insert_batch_parallel(vectors.iter().map(|(id, v)| (*id, v.as_slice())));
    let elapsed = start.elapsed();

    index.set_searching_mode();

    println!("Inserted {inserted}/{NUM_VECTORS} vectors ({DIMENSION}D) in {elapsed:.2?}");
    println!(
        "Throughput: {:.0} vec/s",
        inserted as f64 / elapsed.as_secs_f64() // Reason: usize→f64 is lossless on 64-bit
    );
}
