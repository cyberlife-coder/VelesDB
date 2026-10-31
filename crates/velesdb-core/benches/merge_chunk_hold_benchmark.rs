//! How long a merge into the HNSW index holds the `vector_storage` read
//! guard: the longest a writer to the collection waits on a merge.
//!
//! A merge (the async index builder's flush, the deferred indexer's merge)
//! reads `MERGE_CHUNK` ids' vectors from storage and inserts them into the
//! index under one read guard (`Collection::index_current`). This runs one
//! chunk of that exact code over a 10K-vector 768-d Cosine collection and
//! reports the time between taking the guard and releasing it, as
//! `internal_bench::merge_one_chunk` measures it inside the merge.
//!
//! Needs the `internal-bench` feature:
//! `cargo bench -p velesdb-core --features internal-bench --bench merge_chunk_hold_benchmark`.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use criterion::{criterion_group, criterion_main, Criterion};
use velesdb_core::internal_bench::{merge_one_chunk, unindex, MERGE_CHUNK};
use velesdb_core::{DistanceMetric, Point, VectorCollection};

const VECTOR_COUNT: usize = 10_000;
const DIMENSION: usize = 768;

fn vector(id: u64) -> Vec<f32> {
    (0..DIMENSION)
        .map(|j| ((id as usize * 31 + j * 7) % 1000) as f32 / 1000.0)
        .collect()
}

/// Each iteration stores a fresh chunk (indexed by the upsert), takes it back
/// out of the index while leaving it stored — as a merge finds what its
/// queue holds — and times the merge's guard over that chunk. The graph
/// grows by one chunk's tombstones and nodes per iteration.
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
    for batch in (0..VECTOR_COUNT as u64).collect::<Vec<_>>().chunks(1_000) {
        let points: Vec<Point> = batch
            .iter()
            .map(|&id| Point::without_payload(id, vector(id)))
            .collect();
        coll.upsert_bulk(&points).expect("bench: load");
    }
    let mut next_id = VECTOR_COUNT as u64;

    group.bench_function("chunk_guard_hold_256_into_10k_768d", |b| {
        b.iter_custom(|iters| {
            let mut held = std::time::Duration::ZERO;
            for _ in 0..iters {
                let ids: Vec<u64> = (next_id..next_id + MERGE_CHUNK as u64).collect();
                next_id += MERGE_CHUNK as u64;
                let points: Vec<Point> = ids
                    .iter()
                    .map(|&id| Point::without_payload(id, vector(id)))
                    .collect();
                coll.upsert_bulk(&points).expect("bench: store the chunk");
                unindex(&coll, &ids);
                held += merge_one_chunk(&coll, &ids);
            }
            held
        });
    });

    group.finish();
}

criterion_group!(benches, bench_merge_chunk_hold);
criterion_main!(benches);
