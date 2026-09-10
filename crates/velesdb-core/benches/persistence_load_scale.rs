//! What loading an index costs, and where that cost actually lands.
//!
//! Run with: `cargo bench --bench persistence_load_scale`
//!
//! # Why this exists
//!
//! `persistence_save_scale` measures the write half of the round trip. This is
//! the read half, and it is the half #2173 was about: since #2215 a v2
//! `.vectors` is mapped as the graph's f32 arena instead of being deserialized
//! four bytes at a time. That change shipped with no figure attached, because
//! the save instrument builds its index in memory and therefore never adopts —
//! it cannot see the difference. This file is what makes the claim measurable.
//!
//! # The trap this benchmark exists to avoid
//!
//! **A mapped load does not read the vectors.** It maps them. The pages fault
//! in on first touch, which is a search, not the load call. So a benchmark that
//! times `load` alone reports the adopted path as nearly free and quietly moves
//! the cost outside its own measurement window — flattering, and wrong.
//!
//! Two groups therefore, and both matter:
//!
//! - `hnsw_load` — the load call alone. This is what a caller waits for before
//!   the index is usable, and on the adopted path it genuinely is faster.
//! - `hnsw_load_then_query` — the same load followed by a fixed set of
//!   searches. The deferred faults land inside this window, so it is the
//!   honest apples-to-apples comparison between mapping and copying.
//!
//! Quote the first only next to the second. A gain that appears in one and
//! vanishes in the other is a cost that moved, not a cost that went away.
//!
//! # Sweep dimension, not node count
//!
//! Loading reads `.vectors` and `.graph`. Only the first scales with
//! **dimension**; the graph scales with **node count**. Holding the node count
//! fixed and sweeping dimension therefore leaves the vector payload as the only
//! variable, which is the same reasoning `persistence_save_scale` documents and
//! the only way a difference is attributable to the arena at all.
//!
//! # What decides whether the load adopts
//!
//! Adoption needs a v2 file, a little-endian target, and a store of at least
//! `ContiguousVectors::MIN_ARENA_CAPACITY` vectors. The defaults below clear
//! that floor by a wide margin, but a configuration that does not would measure
//! the copy path while looking like it measured the mapped one — so keep the
//! node count well above it when changing the sizing.
//!
//! # What it measured, and what that number is NOT
//!
//! At 5 000 nodes, Euclidean, two runs per configuration, comparing `develop`
//! against the same tree with `adopt_durable_file` forced to `None`:
//!
//! | 3072d (58.6 MiB payload) | adopted | today's copy path |
//! |---|---|---|
//! | `hnsw_load` | 2.0 / 5.7 ms | 106.5 / 97.7 ms |
//! | `hnsw_load_then_query` | 16.8 / 23.7 ms | 110.9 / 121.7 ms |
//!
//! **That ratio is not the value of mapping.** The copy path it is measured
//! against reads the payload with one `read_exact` per `f32` — 15.36 million
//! calls at this size, about 6.8 ns each, which is `BufReader` call overhead
//! rather than memory bandwidth. It is the read-side twin of the write-side
//! defect fixed in #2212, and it has not been fixed yet.
//!
//! The sibling instrument already measured what a bulk transfer of this exact
//! payload costs: 2 186 MiB/s, so roughly 27 ms for 58.6 MiB. A copy path
//! reading in bulk would land near that, against 16.8-23.7 ms adopted — close
//! to parity. **Most of the measured gap is the per-value read, not the
//! mapping.** Fixing that path and re-running this benchmark is what turns
//! these numbers into an answer about arenas.
//!
//! Three limits belong with the figures:
//!
//! - **Only warm page cache is measured.** The fixture is written immediately
//!   before the group and criterion re-runs on the same files, so every timed
//!   iteration reads the page cache. The "deferred faults" are minor faults,
//!   not disk I/O. Cold behaviour is likely *worse* for the adopted path than
//!   these numbers suggest, because `FileArena` advises `MADV_RANDOM` and so
//!   forgoes kernel readahead that the copy path's `BufReader` enjoys — and
//!   cold is exactly the case that justifies mapping on a constrained device.
//! - **Two runs are not a confidence interval.** The unpaired spread across
//!   these four points is 4.7x to 7.2x; pairing them is arbitrary at n=2.
//!   Criterion's own estimate over its samples is the figure to quote, and it
//!   is printed by the run.
//! - **32 searches do not touch every vector.** An HNSW query visits a subset,
//!   so this captures part of the deferred read. Subtract `hnsw_query_only` to
//!   separate the search compute from the load and its faults.
//!
//! # The decomposition, worked
//!
//! One adopted run at 5 000 x 3072d:
//!
//! | group | time |
//! |---|---|
//! | `hnsw_load` | 5.34 ms |
//! | `hnsw_query_only` | 16.11 ms |
//! | `hnsw_load_then_query` | 24.20 ms |
//!
//! Load plus its deferred faults is `24.20 - 16.11 = 8.09 ms`, of which the
//! load call is 5.34 ms — so the faults are roughly 2.7 ms. **The search
//! compute dominates the combined window**, which is why a ratio taken on
//! `hnsw_load_then_query` alone says more about HNSW than about arenas. That
//! is the arithmetic the control arm exists to make possible.
//!
//! # Sizing
//!
//! Defaults finish unattended. The payload has to dominate the graph-and-
//! sidecar constant before this instrument can arbitrate anything; on a machine
//! that is not idle that starts around 3072 dimensions, which is the calibration
//! `persistence_save_scale` records.
//!
//! ```text
//! VELESDB_LOAD_NODES=20000 \
//! VELESDB_LOAD_DIMS=768,3072 \
//! cargo bench --bench persistence_load_scale
//! ```
//!
//! Build time is sequential insertion and dominates the run: budget roughly a
//! minute per 200K nodes before any measurement starts.

#![allow(
    clippy::cast_precision_loss,
    reason = "synthetic vector values and MiB figures for display: exactness past 2^24 matters to neither"
)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode};
use std::time::Duration;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, HnswIndex, VectorIndex};

/// Vector dimensions to sweep, overridable with `VELESDB_LOAD_DIMS`.
const DEFAULT_DIMS: &str = "768,3072";
/// Node count held constant across the sweep, overridable with
/// `VELESDB_LOAD_NODES`.
const DEFAULT_NODES: usize = 5_000;
/// Searches run after the load in the second and third groups.
const QUERIES: usize = 32;

/// Offset applied to query seeds so they never collide with the `0..nodes`
/// seeds the fixture was built from.
///
/// Derived from the node count at the call site rather than fixed, because
/// `VELESDB_LOAD_NODES` is tunable and the module docs invite raising it: a
/// hard-coded offset silently starts colliding once the fixture grows past it.
fn query_seed(nodes: usize, i: usize) -> u64 {
    (nodes + i) as u64
}

/// Metric used throughout. Euclidean rather than cosine on purpose: the cosine
/// load path re-normalizes vectors outside its epsilon gate, which is work this
/// benchmark is not trying to time.
const METRIC: DistanceMetric = DistanceMetric::Euclidean;

/// Generates a random-ish vector, matching `hnsw_benchmark`'s generator.
///
/// The *inputs* match across this repository's benchmarks; the persisted bytes
/// do not match `persistence_save_scale`'s, which builds in `Cosine` and so
/// stores normalized vectors where this file stores raw ones. Same kind of
/// data, different arena contents — do not read a byte count from one file
/// into the other's numbers.
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| (seed as f32 * 0.1 + i as f32 * 0.01).sin().midpoint(1.0))
        .collect()
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn dimensions() -> Vec<usize> {
    std::env::var("VELESDB_LOAD_DIMS")
        .unwrap_or_else(|_| DEFAULT_DIMS.to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

/// Builds an index of `nodes` vectors and saves it, returning its directory.
///
/// The directory outlives the returned value's use by being handed back: a
/// `TempDir` deletes its contents on drop, and a load benchmark whose fixture
/// vanished measures an error path.
fn persisted_index(nodes: usize, dimension: usize) -> TempDir {
    let index = HnswIndex::new(dimension, METRIC).expect("bench: index");
    for i in 0..nodes as u64 {
        index.insert(i, &generate_vector(dimension, i));
    }
    index.set_searching_mode();
    let home = TempDir::new().expect("bench: temp dir");
    index.save(home.path()).expect("bench: save");
    home
}

fn payload_mib(nodes: usize, dim: usize) -> f64 {
    (nodes * dim * std::mem::size_of::<f32>()) as f64 / (1024.0 * 1024.0)
}

fn bench_load(c: &mut Criterion) {
    let nodes = env_usize("VELESDB_LOAD_NODES", DEFAULT_NODES);
    let mut group = c.benchmark_group("hnsw_load");
    group.sampling_mode(SamplingMode::Flat);
    group.measurement_time(Duration::from_secs(20));
    group.sample_size(10);

    for dim in dimensions() {
        let fixture = persisted_index(nodes, dim);
        println!(
            "  [{nodes} nodes x {dim}d] vector payload {:.1} MiB — load only; on a mapped \
             load the pages have NOT been read yet, so read this next to hnsw_load_then_query \
             and never on its own",
            payload_mib(nodes, dim)
        );
        group.bench_with_input(BenchmarkId::new("load", dim), &dim, |b, _| {
            b.iter(|| {
                let index = HnswIndex::load(fixture.path(), dim, METRIC).expect("bench: load");
                // Asserted, not merely read: a fixture that loaded zero vectors
                // would otherwise time as a very fast load.
                assert_eq!(index.len(), nodes, "bench: fixture did not load");
                black_box(index.len())
            });
        });
    }

    group.finish();
}

fn bench_load_then_query(c: &mut Criterion) {
    let nodes = env_usize("VELESDB_LOAD_NODES", DEFAULT_NODES);
    let mut group = c.benchmark_group("hnsw_load_then_query");
    group.sampling_mode(SamplingMode::Flat);
    group.measurement_time(Duration::from_secs(20));
    group.sample_size(10);

    for dim in dimensions() {
        let fixture = persisted_index(nodes, dim);
        let queries: Vec<Vec<f32>> = (0..QUERIES)
            .map(|i| generate_vector(dim, query_seed(nodes, i)))
            .collect();
        println!(
            "  [{nodes} nodes x {dim}d] vector payload {:.1} MiB — load plus {QUERIES} searches; \
             a mapped load's deferred page faults land inside this window",
            payload_mib(nodes, dim)
        );
        group.bench_with_input(BenchmarkId::new("load_then_query", dim), &dim, |b, _| {
            b.iter(|| {
                let index = HnswIndex::load(fixture.path(), dim, METRIC).expect("bench: load");
                assert_eq!(index.len(), nodes, "bench: fixture did not load");
                for query in &queries {
                    // `VectorIndex::search` swallows its errors and returns an
                    // empty vector, so without this assertion the whole group
                    // could be timing 32 instant failures and reporting them as
                    // a fast load. The deferred page faults this group exists to
                    // capture only happen if the searches actually run.
                    let hits = index.search(query, 10);
                    assert!(!hits.is_empty(), "bench: search returned nothing");
                    black_box(hits);
                }
            });
        });
    }

    group.finish();
}

/// Searches on an index that is already loaded and already faulted in.
///
/// The control arm. `hnsw_load_then_query` sums a load and 32 searches, and the
/// search compute at 3072d is a term common to both sides of any comparison —
/// without measuring it, the deferred page-fault cost cannot be separated from
/// the query cost, and the instrument bounds its own result instead of
/// establishing it. Subtract this from `hnsw_load_then_query` to get the load
/// plus its faults.
fn bench_query_only(c: &mut Criterion) {
    let nodes = env_usize("VELESDB_LOAD_NODES", DEFAULT_NODES);
    let mut group = c.benchmark_group("hnsw_query_only");
    group.sampling_mode(SamplingMode::Flat);
    group.measurement_time(Duration::from_secs(20));
    group.sample_size(10);

    for dim in dimensions() {
        let fixture = persisted_index(nodes, dim);
        let queries: Vec<Vec<f32>> = (0..QUERIES)
            .map(|i| generate_vector(dim, query_seed(nodes, i)))
            .collect();
        // Loaded once, outside the closure, and warmed by a full query pass so
        // the pages are resident before anything is timed.
        let index = HnswIndex::load(fixture.path(), dim, METRIC).expect("bench: load");
        assert_eq!(index.len(), nodes, "bench: fixture did not load");
        for query in &queries {
            assert!(
                !index.search(query, 10).is_empty(),
                "bench: search returned nothing"
            );
        }

        group.bench_with_input(BenchmarkId::new("query_only", dim), &dim, |b, _| {
            b.iter(|| {
                for query in &queries {
                    black_box(index.search(query, 10));
                }
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_load, bench_load_then_query, bench_query_only);
criterion_main!(benches);
