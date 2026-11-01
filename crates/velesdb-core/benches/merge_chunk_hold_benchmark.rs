//! How long a merge into the HNSW index holds the `vector_storage` read
//! guard, which is how long a writer to the collection waits on it.
//!
//! A merge (the async index builder's flush, the deferred indexer's merge)
//! reads `MERGE_CHUNK` ids' vectors from storage and inserts them into the
//! index under one read guard (`Collection::index_current`). This runs one
//! chunk of that exact code over a 10 000-point 768-d Cosine collection and
//! reports the time between taking the guard and releasing it, as
//! `internal_bench::merge_one_chunk` measures it inside the merge.
//!
//! The chunk is the collection's first `MERGE_CHUNK` points, taken back out
//! of the index before each merge and merged back in, so the collection
//! holds 10 000 points throughout. Each removal leaves its nodes in the
//! graph, unmapped, until a vacuum: the graph gains `MERGE_CHUNK` such nodes
//! per iteration.
//!
//! Needs the `internal-bench` feature:
//! `cargo bench -p velesdb-core --features internal-bench --bench merge_chunk_hold_benchmark`.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use criterion::{criterion_group, criterion_main, Criterion};
use velesdb_core::internal_bench::{merge_one_chunk, unindex, MERGE_CHUNK};
use velesdb_core::{DistanceMetric, Point, VectorCollection};

const VECTOR_COUNT: u64 = 10_000;
const DIMENSION: usize = 768;

/// A distinct vector per id: a splitmix64 hash of `(id, dimension)`, mapped
/// to `[-1, 1)`.
fn vector(id: u64) -> Vec<f32> {
    (0..DIMENSION as u64)
        .map(|j| {
            let mut x = id
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(j.wrapping_mul(0xBF58_476D_1CE4_E5B9));
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^= x >> 31;
            (x >> 40) as f32 / (1_u64 << 24) as f32 * 2.0 - 1.0
        })
        .collect()
}

fn bench_merge_chunk_hold(c: &mut Criterion) {
    let mut group = c.benchmark_group("merge");
    group.sample_size(10);

    let dir = tempfile::tempdir().expect("bench: temp dir");
    let coll = VectorCollection::create(
        dir.path().join("merge"),
        "merge",
        DIMENSION,
        DistanceMetric::Cosine,
        velesdb_core::StorageMode::Full,
    )
    .expect("bench: collection");
    for batch in (0..VECTOR_COUNT).collect::<Vec<_>>().chunks(1_000) {
        let points: Vec<Point> = batch
            .iter()
            .map(|&id| Point::without_payload(id, vector(id)))
            .collect();
        coll.upsert_bulk(&points).expect("bench: load");
    }
    let chunk: Vec<u64> = (0..MERGE_CHUNK as u64).collect();

    group.bench_function("chunk_guard_hold_256_of_10k_768d", |b| {
        b.iter_custom(|iters| {
            let mut held = std::time::Duration::ZERO;
            for _ in 0..iters {
                unindex(&coll, &chunk);
                held += merge_one_chunk(&coll, &chunk);
            }
            held
        });
    });

    group.finish();
}

criterion_group!(benches, bench_merge_chunk_hold);
criterion_main!(benches);
